//! Transport for explicitly opened, runtime-owned user terminals.
use base64::{engine::general_purpose::STANDARD, Engine};
use chatspeed_contracts::{
    TerminalCreateRequest, TerminalResizeRequest, TerminalStreamEvent, TerminalWriteRequest,
};
use chatspeed_runtime_client::LeaseGuard;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

use crate::{native, types::Options};

pub enum Input {
    Text(String),
    Resize(u16, u16),
}

pub enum Output {
    Shells(Vec<(String, String)>),
    Ready(String),
    Bytes(Vec<u8>),
    Exit,
    Lost,
}

pub async fn run(
    options: Options,
    cwd: Option<String>,
    shell_path: Option<String>,
    size: (u16, u16),
    mut input: mpsc::Receiver<Input>,
    output: mpsc::Sender<Output>,
) {
    let result = connect(&options, cwd, shell_path, size, &mut input, &output).await;
    if result.is_err() {
        let _ = output.send(Output::Lost).await;
    }
}

async fn connect(
    options: &Options,
    cwd: Option<String>,
    shell_path: Option<String>,
    size: (u16, u16),
    input: &mut mpsc::Receiver<Input>,
    output: &mpsc::Sender<Output>,
) -> Result<(), ()> {
    if options.demo {
        return Err(());
    }
    let client = native::connect(options).await.ok_or(())?;
    if input.is_closed() {
        return Ok(());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?
        .as_nanos();
    let id = format!("gpui-terminal-{}-{nonce}", std::process::id());
    let mut lease = LeaseGuard::register(&client, &id, "gpui")
        .await
        .map_err(|_| ())?;
    let heartbeat = lease.heartbeat();
    let proof = lease.lease_id().to_owned();
    let result = async {
        if input.is_closed() { return Ok(()); }
        let shells = client.terminal_list_shells(&id, &proof).await.unwrap_or_default();
        let _ = output.send(Output::Shells(shells.iter().map(|shell| (shell.name.clone(), shell.path.clone())).collect())).await;
        if input.is_closed() { return Ok(()); }
        if shell_path.as_ref().is_some_and(|path| !shells.iter().any(|shell| &shell.path == path)) { return Err(()); }
        let tab = client.terminal_create(&id, &proof, &TerminalCreateRequest { cwd, cols: Some(size.0), rows: Some(size.1), shell_path }).await.map_err(|_| ())?;
        let session = tab.session_id;
        let result = async {
            let mut stream = tokio::time::timeout(Duration::from_secs(5), client.terminal_stream(&id, &proof, &session)).await.map_err(|_| ())?.map_err(|_| ())?;
            output.send(Output::Ready(tab.shell_name)).await.map_err(|_| ())?;
            // A separate FIFO prevents a slow write from stalling the SSE reader.
            let writes = async {
                while let Some(action) = input.recv().await {
                    match action {
                        Input::Text(input) => client.terminal_write(&id, &proof, &session, &TerminalWriteRequest { input }).await,
                        Input::Resize(cols, rows) => client.terminal_resize(&id, &proof, &session, &TerminalResizeRequest { cols, rows }).await,
                    }.map_err(|_| ())?;
                }
                Ok(())
            };
            let reads = async {
                let mut previous = None;
                loop {
                    let envelope = stream.next_event().await.map_err(|_| ())?.ok_or(())?;
                    if envelope.session_id != session || previous.is_some_and(|n: u64| n.checked_add(1) != Some(envelope.sequence)) { return Err(()); }
                    previous = Some(envelope.sequence);
                    match envelope.event {
                        TerminalStreamEvent::Output(chunk) if chunk.session_id == session => {
                            let bytes = STANDARD.decode(chunk.data_base64).map_err(|_| ())?;
                            output.send(Output::Bytes(bytes)).await.map_err(|_| ())?;
                        }
                        TerminalStreamEvent::Exit(exit) if exit.session_id == session => {
                            output.send(Output::Exit).await.map_err(|_| ())?;
                            return Ok(());
                        }
                        _ => return Err(()),
                    }
                }
            };
            tokio::select! { result = writes => result, result = reads => result, _ = output.closed() => Ok(()) }
        }.await;
        let _ = tokio::time::timeout(Duration::from_millis(750), client.terminal_close(&id, &proof, &session)).await;
        result
    }.await;
    drop(heartbeat);
    let _ = tokio::time::timeout(Duration::from_millis(750), lease.release()).await;
    result
}
