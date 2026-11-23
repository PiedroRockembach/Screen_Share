use anyhow::{Context, Result};
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType, StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode};
use std::os::fd::OwnedFd;

pub struct ScreenCapture {
    pub fd: OwnedFd,
    pub node: u32,
    _session: Box<dyn std::any::Any + Send>,
}

/// Asks the desktop (KDE/GNOME) for a monitor. With a saved restore token the
/// dialog is skipped and the same monitor is reused.
pub async fn request(restore_token: Option<String>) -> Result<(ScreenCapture, Option<String>)> {
    let proxy = Screencast::new().await.context("o sistema não oferece captura de tela (portal)")?;
    let session = proxy.create_session(CreateSessionOptions::default()).await?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::EMBEDDED)
                .set_sources(SourceType::MONITOR)
                .set_multiple(false)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(restore_token.as_deref()),
        )
        .await?
        .response()?;
    let streams = proxy
        .start(&session, None, StartCastOptions::default())
        .await?
        .response()
        .context("a captura da tela foi cancelada")?;
    let node = streams.streams().first().context("nenhuma tela foi escolhida")?.pipe_wire_node_id();
    let token = streams.restore_token().map(str::to_owned);
    let fd = proxy.open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default()).await?;
    Ok((ScreenCapture { fd, node, _session: Box::new((proxy, session)) }, token))
}
