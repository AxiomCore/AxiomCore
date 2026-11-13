//! `axiom login`: the device flow, in the full-screen TUI when the terminal can
//! show it and as plain text otherwise (B11). See [`super::login_plain`].

use super::login_plain::{self, DeviceStart, LoginMode, TerminalFacts, Timing};
use anyhow::Result;
use crossterm::event::KeyCode;

pub async fn handle_login(no_tui: bool) -> Result<()> {
    let terminal = TerminalFacts::detect();
    let base_url = axiom_cloud::cloud_base_url()?;
    let client = login_plain::http_client()?;
    // Do not enter raw/alternate-screen mode until the device flow is ready. If the
    // authentication service rejects or rate-limits the request, the developer gets
    // a normal shell error and no detached TUI task is left running.
    let start = login_plain::start_device_login(&client, &base_url).await?;
    match login_plain::choose_mode(no_tui, terminal) {
        LoginMode::Plain => login_plain::run_plain_with(&client, &base_url, &start, terminal).await,
        LoginMode::Tui => match handle_login_tui(&client, &base_url, &start, terminal).await {
            Err(TuiError::Unavailable) => {
                // The terminal refused raw mode or the alternate screen: the
                // same device code still works in plain text.
                login_plain::run_plain_with(&client, &base_url, &start, terminal).await
            }
            Err(TuiError::Login(error)) => Err(error),
            Ok(()) => Ok(()),
        },
    }
}

enum TuiError {
    /// The TUI could not start; nothing was drawn and polling has not begun.
    Unavailable,
    Login(anyhow::Error),
}

async fn handle_login_tui(
    client: &reqwest::Client,
    base_url: &str,
    start: &DeviceStart,
    terminal: TerminalFacts,
) -> std::result::Result<(), TuiError> {
    let mut tui = crate::tui::Tui::new().map_err(|_| TuiError::Unavailable)?;
    if tui.enter().is_err() {
        let _ = tui.exit();
        return Err(TuiError::Unavailable);
    }

    let mut state = crate::state::State::new();
    // UX-9: the prefilled link when the server sends one; the code stays visible
    // so it can be compared with the browser page.
    state.login_context.status = crate::state::LoginStatus::WaitingForUser {
        code: start.user_code.clone(),
        url: start.link().to_string(),
    };
    if login_plain::should_open_browser(start, terminal) {
        login_plain::open_browser(start.link());
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let login_task = {
        let client = client.clone();
        let base_url = base_url.to_string();
        let start = start.clone();
        tokio::spawn(async move {
            let result =
                login_plain::poll_for_session(&client, &base_url, &start, Timing::PRODUCTION).await;
            let _ = tx.send(result);
        })
    };

    let loop_result: Result<()> = async {
        loop {
            if let Ok(result) = rx.try_recv() {
                match result.and_then(|session| login_plain::save_session(&session)) {
                    Ok(()) => state.login_context.status = crate::state::LoginStatus::Success,
                    Err(e) => {
                        state.login_context.status =
                            crate::state::LoginStatus::Error(e.to_string());
                    }
                }
            }

            tui.draw(|f| {
                crate::components::login_screen::render_login_screen(f, f.size(), &state)
            })?;

            if let Some(crate::tui::Event::Key(key)) = tui.event_rx.recv().await {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        login_task.abort();
                        if let crate::state::LoginStatus::Error(message) =
                            &state.login_context.status
                        {
                            return Err(anyhow::anyhow!(message.clone()));
                        }
                        return Ok(());
                    }
                    KeyCode::Enter
                        if matches!(
                            state.login_context.status,
                            crate::state::LoginStatus::Success
                        ) =>
                    {
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
    .await;

    let exited = tui.exit().map_err(|e| anyhow::anyhow!(e));
    loop_result.and(exited).map_err(TuiError::Login)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn login_accepts_no_tui() {
        let cli = crate::Cli::try_parse_from(["axiom", "login", "--no-tui"]).unwrap();
        assert!(matches!(
            *cli.command,
            crate::Commands::Login {
                step_up: false,
                no_tui: true
            }
        ));
        let cli = crate::Cli::try_parse_from(["axiom", "login"]).unwrap();
        assert!(matches!(
            *cli.command,
            crate::Commands::Login { no_tui: false, .. }
        ));
    }
}
