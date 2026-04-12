//! Runtime-owned shared terminal process.
//!
//! Slice 08 moves terminal lifetime above transient ACP transport attachments.
//! `SharedTerminal` owns one subprocess and exposes single-attachment borrows
//! that are served through the ACP SDK's normal transport primitives.

use std::io;
use std::sync::Arc;

use anyhow::{Context, Result};
use fireline_tools::agent_catalog::resolve_agent_launch_command;
use sacp::{Client, ConnectTo, Lines};
use sacp_tokio::AcpAgent;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, mpsc, oneshot};

#[derive(Clone)]
pub struct SharedTerminal {
    attach_tx: mpsc::Sender<AttachRequest>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachError {
    Busy,
    Closed,
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("runtime_busy"),
            Self::Closed => f.write_str("runtime_closed"),
        }
    }
}

impl std::error::Error for AttachError {}

struct AttachRequest {
    reply_tx: oneshot::Sender<Result<SharedTerminalAttachment, AttachError>>,
}

struct ActorAttachment {
    id: u64,
    to_conductor_tx: mpsc::Sender<io::Result<String>>,
}

enum ActorMessage {
    WriteLine { attachment_id: u64, line: String },
    Detached { attachment_id: u64 },
}

pub struct SharedTerminalAttachment {
    outgoing_tx: mpsc::Sender<String>,
    incoming_rx: Option<mpsc::Receiver<io::Result<String>>>,
    detached_tx: Option<oneshot::Sender<()>>,
}

impl SharedTerminal {
    pub async fn spawn(agent_command: Vec<String>) -> Result<Self> {
        let resolved_agent_command = resolve_agent_launch_command(agent_command)
            .await
            .context("resolve agent command for shared terminal launch")?;
        let agent = AcpAgent::from_args(resolved_agent_command)
            .map_err(|e| anyhow::anyhow!("agent command: {e}"))?;
        let (stdin, stdout, stderr, child) = agent
            .spawn_process()
            .map_err(anyhow::Error::from)
            .context("spawn shared terminal subprocess")?;

        let (attach_tx, attach_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            SharedTerminalActor::new(stdin, stdout, stderr, child, attach_rx, shutdown_rx)
                .run()
                .await;
        });

        Ok(Self {
            attach_tx,
            shutdown: Arc::new(Mutex::new(Some(shutdown_tx))),
            task: Arc::new(Mutex::new(Some(task))),
        })
    }

    pub async fn try_attach(&self) -> Result<SharedTerminalAttachment, AttachError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.attach_tx
            .send(AttachRequest { reply_tx })
            .await
            .map_err(|_| AttachError::Closed)?;

        reply_rx.await.unwrap_or(Err(AttachError::Closed))
    }

    pub async fn shutdown(&self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown.lock().await.take() {
            let _ = shutdown_tx.send(());
        }

        if let Some(task) = self.task.lock().await.take() {
            task.await.map_err(anyhow::Error::from)?;
        }

        Ok(())
    }
}

impl ConnectTo<Client> for SharedTerminalAttachment {
    async fn connect_to(mut self, client: impl ConnectTo<sacp::Agent>) -> Result<(), sacp::Error> {
        let outgoing = futures::sink::unfold(
            self.outgoing_tx.clone(),
            |sender, line: String| async move {
                sender.send(line).await.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "shared terminal closed")
                })?;
                Ok::<_, io::Error>(sender)
            },
        );

        let incoming_rx = self
            .incoming_rx
            .take()
            .expect("shared terminal attachment incoming receiver missing");
        let incoming = futures::stream::unfold(incoming_rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });

        sacp::ConnectTo::<Client>::connect_to(Lines::new(outgoing, incoming), client).await
    }
}

impl Drop for SharedTerminalAttachment {
    fn drop(&mut self) {
        if let Some(detached_tx) = self.detached_tx.take() {
            let _ = detached_tx.send(());
        }
    }
}

struct SharedTerminalActor {
    stdin: ChildStdin,
    stdout_lines: tokio::io::Lines<BufReader<ChildStdout>>,
    child: Child,
    attach_rx: mpsc::Receiver<AttachRequest>,
    actor_tx: mpsc::Sender<ActorMessage>,
    actor_rx: mpsc::Receiver<ActorMessage>,
    shutdown_rx: oneshot::Receiver<()>,
    current: Option<ActorAttachment>,
    next_attachment_id: u64,
    stderr_task: tokio::task::JoinHandle<()>,
}

impl SharedTerminalActor {
    fn new(
        stdin: ChildStdin,
        stdout: ChildStdout,
        stderr: ChildStderr,
        child: Child,
        attach_rx: mpsc::Receiver<AttachRequest>,
        shutdown_rx: oneshot::Receiver<()>,
    ) -> Self {
        let (actor_tx, actor_rx) = mpsc::channel(32);
        let stderr_task = tokio::spawn(drain_stderr(stderr));

        Self {
            stdin,
            stdout_lines: BufReader::new(stdout).lines(),
            child,
            attach_rx,
            actor_tx,
            actor_rx,
            shutdown_rx,
            current: None,
            next_attachment_id: 0,
            stderr_task,
        }
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                request = self.attach_rx.recv() => {
                    let Some(request) = request else {
                        break;
                    };
                    let _ = request.reply_tx.send(self.try_install_attachment());
                }
                actor_message = self.actor_rx.recv() => {
                    let Some(actor_message) = actor_message else {
                        break;
                    };
                    self.handle_actor_message(actor_message).await;
                }
                line = self.stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => self.forward_stdout(line).await,
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(error = %error, "shared terminal stdout read failed");
                            break;
                        }
                    }
                }
                _ = &mut self.shutdown_rx => {
                    let _ = self.child.start_kill();
                    break;
                }
            }
        }

        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        self.stderr_task.abort();
    }

    fn try_install_attachment(&mut self) -> Result<SharedTerminalAttachment, AttachError> {
        if self.current.is_some() {
            return Err(AttachError::Busy);
        }

        self.next_attachment_id += 1;
        let attachment_id = self.next_attachment_id;

        let (outgoing_tx, outgoing_rx) = mpsc::channel::<String>(32);
        let (incoming_tx, incoming_rx) = mpsc::channel::<io::Result<String>>(32);
        let (detached_tx, detached_rx) = oneshot::channel();

        self.current = Some(ActorAttachment {
            id: attachment_id,
            to_conductor_tx: incoming_tx,
        });

        let actor_tx = self.actor_tx.clone();
        tokio::spawn(async move {
            drive_attachment_input(attachment_id, outgoing_rx, detached_rx, actor_tx).await;
        });

        Ok(SharedTerminalAttachment {
            outgoing_tx,
            incoming_rx: Some(incoming_rx),
            detached_tx: Some(detached_tx),
        })
    }

    async fn handle_actor_message(&mut self, actor_message: ActorMessage) {
        match actor_message {
            ActorMessage::WriteLine {
                attachment_id,
                line,
            } => {
                if self.current.as_ref().map(|current| current.id) != Some(attachment_id) {
                    return;
                }

                let mut bytes = line.into_bytes();
                bytes.push(b'\n');
                if let Err(error) = self.stdin.write_all(&bytes).await {
                    tracing::warn!(error = %error, "shared terminal stdin write failed");
                    self.current = None;
                    return;
                }
                if let Err(error) = self.stdin.flush().await {
                    tracing::warn!(error = %error, "shared terminal stdin flush failed");
                    self.current = None;
                }
            }
            ActorMessage::Detached { attachment_id } => {
                if self.current.as_ref().map(|current| current.id) == Some(attachment_id) {
                    self.current = None;
                }
            }
        }
    }

    async fn forward_stdout(&mut self, line: String) {
        let Some(current) = self.current.as_ref() else {
            return;
        };

        if current.to_conductor_tx.send(Ok(line)).await.is_err() {
            self.current = None;
        }
    }
}

async fn drive_attachment_input(
    attachment_id: u64,
    mut outgoing_rx: mpsc::Receiver<String>,
    mut detached_rx: oneshot::Receiver<()>,
    actor_tx: mpsc::Sender<ActorMessage>,
) {
    loop {
        tokio::select! {
            line = outgoing_rx.recv() => {
                let Some(line) = line else {
                    break;
                };
                if actor_tx.send(ActorMessage::WriteLine { attachment_id, line }).await.is_err() {
                    break;
                }
            }
            _ = &mut detached_rx => {
                break;
            }
        }
    }

    let _ = actor_tx
        .send(ActorMessage::Detached { attachment_id })
        .await;
}

async fn drain_stderr(stderr: ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => tracing::debug!(line, "shared terminal stderr"),
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(error = %error, "shared terminal stderr drain failed");
                break;
            }
        }
    }
}
