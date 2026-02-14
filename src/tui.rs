use color_eyre::eyre::Result;
use crossterm::{
    cursor,
    event::{Event as CrosstermEvent, KeyEvent, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::{FutureExt, StreamExt};
use ratatui::backend::CrosstermBackend as Backend;
use std::ops::{Deref, DerefMut};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub type IO = std::io::Stdout;
pub type Frame<'a> = ratatui::Frame<'a>;

pub struct Tui {
    pub terminal: ratatui::Terminal<Backend<IO>>,
    pub task: JoinHandle<()>,
    pub cancellation_token: CancellationToken,
    pub event_rx: mpsc::UnboundedReceiver<Event>,
    pub event_tx: mpsc::UnboundedSender<Event>,
}

#[derive(Clone, Debug)]
pub enum Event {
    Tick,
    Render,
    Key(KeyEvent),
    Resize(u16, u16),
}

impl Tui {
    pub fn new() -> Result<Self> {
        let terminal = ratatui::Terminal::new(Backend::new(std::io::stdout()))?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let cancellation_token = CancellationToken::new();
        let task = tokio::spawn(async {});
        Ok(Self {
            terminal,
            task,
            cancellation_token,
            event_rx,
            event_tx,
        })
    }

    pub fn start(&mut self) {
        let tick_delay = std::time::Duration::from_millis(250);
        let render_delay = std::time::Duration::from_millis(16); // ~60fps
        self.cancellation_token = CancellationToken::new();
        let _cancellation_token = self.cancellation_token.clone();
        let _event_tx = self.event_tx.clone();

        self.task = tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick_interval = tokio::time::interval(tick_delay);
            let mut render_interval = tokio::time::interval(render_delay);
            loop {
                tokio::select! {
                    _ = _cancellation_token.cancelled() => break,
                    maybe_event = reader.next().fuse() => {
                        if let Some(Ok(CrosstermEvent::Key(key))) = maybe_event {
                            if key.kind == KeyEventKind::Press {
                                _event_tx.send(Event::Key(key)).unwrap();
                            }
                        }
                    },
                    _ = tick_interval.tick() => _event_tx.send(Event::Tick).unwrap(),
                    _ = render_interval.tick() => _event_tx.send(Event::Render).unwrap(),
                }
            }
        });
    }

    pub fn enter(&mut self) -> Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), EnterAlternateScreen, cursor::Hide)?;
        self.start();
        Ok(())
    }

    pub fn exit(&mut self) -> Result<()> {
        self.cancellation_token.cancel();
        crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, cursor::Show)?;
        crossterm::terminal::disable_raw_mode()?;
        Ok(())
    }
}

impl Deref for Tui {
    type Target = ratatui::Terminal<Backend<IO>>;
    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

impl Tui {
    pub fn stop_and_clear(&mut self) -> Result<()> {
        self.exit()?;
        // Optional: clear screen or move cursor back
        Ok(())
    }
}
