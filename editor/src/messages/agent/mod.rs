mod agent_message;
mod agent_message_handler;

#[doc(inline)]
pub use agent_message::{AgentMessage, AgentMessageDiscriminant};
#[doc(inline)]
pub use agent_message_handler::{AgentMessageContext, AgentMessageHandler, AgentReplySink};

#[cfg(test)]
mod test;
