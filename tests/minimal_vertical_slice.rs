use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use durable_streams::{Client as DsClient, Offset};
use fireline::stream_host::build_stream_router;
use fireline_conductor::{build::build_subprocess_conductor, trace::DurableStreamTracer};
use tokio::io::duplex;
use tokio::net::TcpListener;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

fn testy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fireline-testy"))
}

#[tokio::test]
async fn minimal_vertical_slice_prompts_and_emits_state_events() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let stream_server = tokio::spawn(async move {
        axum::serve(listener, build_stream_router()?)
            .await
            .map_err(anyhow::Error::from)
    });

    let stream_name = format!("minimal-vertical-slice-{}", uuid::Uuid::new_v4());
    let stream_url = format!("http://{addr}/v1/stream/{stream_name}");
    let runtime_id = "minimal-vertical-slice";

    let client = DsClient::new();
    let stream = client.stream(&stream_url);
    stream.create().await?;

    let producer = stream.producer("state-writer").build();
    let tracer =
        DurableStreamTracer::new(producer.clone(), runtime_id, "conn:minimal-vertical-slice");

    let conductor = build_subprocess_conductor(
        "fireline-test",
        vec![testy_bin().display().to_string()],
        vec![],
        tracer,
    );

    let (client_stream, conductor_stream) = duplex(16 * 1024);
    let conductor_task = tokio::spawn(async move {
        fireline_conductor::transports::duplex::handle_duplex(conductor, conductor_stream).await
    });

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
        body.contains("\"type\":\"connection\""),
        "state stream should contain connection rows: {body}"
    );
    assert!(
        body.contains("\"type\":\"prompt_turn\""),
        "state stream should contain prompt turns: {body}"
    );
    assert!(
        body.contains("\"type\":\"pending_request\""),
        "state stream should contain pending request rows: {body}"
    );
    assert!(
        body.contains("\"type\":\"session\""),
        "state stream should contain session rows: {body}"
    );

    conductor_task.abort();
    stream_server.abort();

    Ok(())
}
