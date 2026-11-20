//! w3-d2 (X10b): choose the serving session for `axiom serve` and `axiom
//! mock`. Without `--inspect-ownership` it is always the plain
//! `real_session`, which never publishes inspection data. With it, a CLI
//! built with the `dev-inspect-ownership` feature starts the development
//! session that publishes the ownership view to the project's Inspector;
//! any other build refuses. `axiom server run`, `axiom server run-bundle`
//! and `axiom database dev` have no such flag and always use `real_session`.
use anyhow::Result;
use axiom_server::{development::PreparedSession, host::ActionHost};
use std::{net::SocketAddr, path::Path, sync::Arc};

/// Whether this CLI build contains the development ownership publisher.
pub const PUBLISHER_COMPILED: bool = cfg!(feature = "dev-inspect-ownership");

pub fn session(
    host: Arc<ActionHost>,
    listen: SocketAddr,
    debug: bool,
    inspect_ownership: Option<&Path>,
) -> Result<PreparedSession> {
    let Some(project) = inspect_ownership else {
        return Ok(axiom_server::development::real_session(host, listen, debug));
    };
    #[cfg(feature = "dev-inspect-ownership")]
    {
        axiom_server::development::development_session_with_ownership(host, listen, debug, project)
            .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))
    }
    #[cfg(not(feature = "dev-inspect-ownership"))]
    {
        let _ = (host, listen, debug, project);
        anyhow::bail!(
            "--inspect-ownership needs a developer build of the CLI (`cargo build --features dev-inspect-ownership`); this build never publishes inspection data"
        )
    }
}

#[cfg(test)]
mod tests {
    /// A build without the feature refuses the flag instead of silently
    /// serving without inspection; with the feature, the server decides.
    #[test]
    fn the_flag_needs_the_feature() {
        if super::PUBLISHER_COMPILED {
            return;
        }
        let source = include_str!("ownership_session.rs");
        assert!(source.contains("needs a developer build of the CLI"));
    }

    /// `server run`, `run-bundle` and `database dev` always use the plain
    /// real session (no publisher), whatever the build.
    #[test]
    fn production_commands_use_the_plain_real_session() {
        let server = include_str!("server.rs");
        assert!(!server.contains("ownership_session"));
        assert!(!server.contains("development_session_with_ownership"));
        assert_eq!(server.matches("development::real_session(").count(), 2);
        let database = include_str!("database_dev.rs");
        assert!(!database.contains("development_session_with_ownership"));
        for (name, source) in [
            ("serve.rs", include_str!("serve.rs")),
            ("mock.rs", include_str!("mock.rs")),
        ] {
            assert!(
                !source.contains("development::real_session("),
                "{name} must choose through ownership_session"
            );
            assert!(source.contains("ownership_session::session("), "{name}");
        }
    }
}
