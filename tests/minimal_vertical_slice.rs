use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline_harness::DurableStreamTracer;
use fireline_session::build_stream_router;
use sacp::{Agent, ByteStreams, Client, Conductor, DynConnectTo};
use sacp_conductor::{ConductorImpl, McpBridgeMode, trace::WriteEvent};
use sacp_tokio::AcpAgent;
use tokio::io::duplex;
use tokio::net::TcpListener;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

fn testy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
}

fn build_subprocess_conductor(
    name: impl ToString,
    agent_command: Vec<String>,
    components: Vec<DynConnectTo<Conductor>>,
    trace_writer: impl WriteEvent,
) -> ConductorImpl<Agent> {
    ConductorImpl::new_agent(
        name,
        move |req| async move {
            let terminal = DynConnectTo::<Client>::new(
                AcpAgent::from_args(agent_command)
                    .map_err(|e| sacp::util::internal_error(format!("agent command: {e}")))?,
            );
            Ok((req, components, terminal))
        },
        McpBridgeMode::default(),
    )
    .trace_to(trace_writer)
}

async fn handle_duplex(
    conductor: ConductorImpl<Agent>,
    stream: tokio::io::DuplexStream,
) -> Result<()> {
    let (read_half, write_half) = tokio::io::split(stream);
    conductor
        .run(ByteStreams::new(
            write_half.compat_write(),
            read_half.compat(),
        ))
        .await?;
    Ok(())
}

#[tokio::test]
async fn minimal_vertical_slice_prompts_and_emits_state_events() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let stream_server = tokio::spawn(async move {
        axum::serve(listener, build_stream_router(None)?)
            .await
            .map_err(anyhow::Error::from)
    });

    let stream_name = format!("minimal-vertical-slice-{}", uuid::Uuid::new_v4());
    let stream_url = format!("http://{addr}/v1/stream/{stream_name}");
    let host_id = "minimal-vertical-slice";

    let client = DsClient::new();
    let stream = client.stream(&stream_url);
    stream.create().await?;

    let producer = stream.producer("state-writer").build();
    let tracer =
        DurableStreamTracer::new(producer.clone(), host_id, "conn:minimal-vertical-slice");

    let conductor = build_subprocess_conductor(
        "fireline-test",
        vec![testy_bin().display().to_string()],
        vec![],
        tracer,
    );

    let (client_stream, conductor_stream) = duplex(16 * 1024);
    let conductor_task =
        tokio::spawn(async move { handle_duplex(conductor, conductor_stream).await });

    let (client_read, client_write) = tokio::io::split(client_stream);
    let response = yopo::prompt(
        sacp::ByteStreams::new(client_write.compat_write(), client_read.compat()),
        "hello from fireline",
    )
    .await?;

    assert_eq!(
        response, "Hello, world!",
        "fireline-testy should respond through the SDK test agent"
    );

    producer.flush().await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut reader = stream.read().offset(Offset::Beginning).build()?;
    let mut body = String::new();
    while let Some(chunk) = reader.next_chunk().await? {
        body.push_str(std::str::from_utf8(&chunk.data)?);
        if chunk.up_to_date {
            break;
        }
    }

    assert!(
        body.contains("\"type\":\"prompt_request\""),
        "state stream should contain canonical prompt_request rows: {body}"
    );
    assert!(
        body.contains("\"type\":\"session_v2\""),
        "state stream should contain canonical session_v2 rows: {body}"
    );
    assert!(
        body.contains("\"type\":\"chunk_v2\""),
        "state stream should contain canonical chunk_v2 rows: {body}"
    );

    conductor_task.abort();
    stream_server.abort();

    Ok(())
}
