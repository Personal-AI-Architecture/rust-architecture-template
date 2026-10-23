use super::{Message, ToolDefinition};
use std::collections::HashMap;

pub type ContextLimits = HashMap<String, HashMap<String, usize>>;

pub fn default_limits() -> ContextLimits {
    HashMap::from([("local".into(), HashMap::from([("mock".into(), 8192)]))])
}

pub fn limit_for(limits: &ContextLimits, provider: &str, model: &str) -> Option<usize> {
    limits
        .get(provider)?
        .get(model)
        .copied()
        .filter(|limit| *limit > 0)
}

/// UTF-8 bytes conservatively upper-bound ordinary token counts, with
/// response, bootstrap and protocol space reserved separately.
pub fn context_cost(messages: &[Message], tools: &[ToolDefinition]) -> usize {
    serde_json::to_vec(&serde_json::json!({"messages": messages, "tools": tools}))
        .map(|bytes| bytes.len() + 384)
        .unwrap_or(usize::MAX)
}

pub fn select_complete_turns(
    messages: &[Message],
    tools: &[ToolDefinition],
    limit: usize,
) -> Option<(Vec<Message>, usize)> {
    let mut turns: Vec<Vec<Message>> = Vec::new();
    for message in messages {
        if message.role == "user" || turns.is_empty() {
            turns.push(Vec::new());
        }
        turns.last_mut().expect("turn").push(message.clone());
    }
    let mut selected = Vec::new();
    let mut kept = 0;
    for turn in turns.iter().rev() {
        let mut candidate = turn.clone();
        candidate.extend(selected.iter().cloned());
        if context_cost(&candidate, tools) > limit {
            if kept == 0 {
                return None;
            }
            break;
        }
        selected = candidate;
        kept += 1;
    }
    Some((selected, turns.len() - kept))
}
