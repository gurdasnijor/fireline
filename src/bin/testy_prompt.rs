use anyhow::Result;
use sacp::schema::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, SessionId,
    SessionNotification, SessionUpdate, StopReason, TextContent,
};
use sacp::{Agent, Client, ConnectTo, ConnectionTo, Responder};

#[derive(Clone, Debug, Default)]
struct PromptEchoAgent;

impl PromptEchoAgent {
    async fn process_prompt(
        &self,
        request: PromptRequest,
        responder: Responder<PromptResponse>,
        connection: ConnectionTo<Client>,
    ) -> Result<(), sacp::Error> {
        let text = request
            .prompt
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(TextContent { text, .. }) => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        connection.send_notification(SessionNotification::new(
            request.session_id,
            SessionUpdate::AgentMessageChunk(ContentChunk::new(text.into())),
        ))?;
        responder.respond(PromptResponse::new(StopReason::EndTurn))
    }
}

impl ConnectTo<Client> for PromptEchoAgent {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), sacp::Error> {
        Agent
            .builder()
            .name("fireline-testy-prompt")
            .on_receive_request(
                async |initialize: InitializeRequest, responder, _cx| {
                    responder.respond(
                        InitializeResponse::new(initialize.protocol_version)
                            .agent_capabilities(AgentCapabilities::new().load_session(false)),
                    )
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                async move |_request: NewSessionRequest, responder, _cx| {
                    responder.respond(NewSessionResponse::new(SessionId::new(
                        uuid::Uuid::new_v4().to_string(),
                    )))
                },
                sacp::on_receive_request!(),
            )
            .on_receive_request(
                {
                    let agent = self.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let cx_clone = cx.clone();
                        cx.spawn({
                            let agent = agent.clone();
                            async move { agent.process_prompt(request, responder, cx_clone).await }
                        })
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    PromptEchoAgent.connect_to(sacp_tokio::Stdio::new()).await?;
    Ok(())
}
