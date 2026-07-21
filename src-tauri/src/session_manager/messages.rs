use std::path::Path;

use super::providers::ProviderRegistry;
use super::types::{QaPair, SessionDetail, SessionMessage};

pub fn load_messages(
    registry: &ProviderRegistry,
    provider_id: &str,
    source_path: &str,
) -> Result<Vec<SessionMessage>, String> {
    let path = Path::new(source_path);
    registry.get(provider_id)?.load_messages(path)
}

fn load_raw_content_fallback(
    registry: &ProviderRegistry,
    provider_id: &str,
    source_path: &str,
) -> Result<Option<String>, String> {
    let path = Path::new(source_path);
    registry.get(provider_id)?.load_raw_content_fallback(path)
}

pub fn load_session_detail(
    registry: &ProviderRegistry,
    provider_id: &str,
    source_path: &str,
) -> Result<SessionDetail, String> {
    let messages = load_messages(registry, provider_id, source_path)?;
    let qa_pairs = extract_qa_pairs(&messages);
    let raw_content = if messages.is_empty() {
        load_raw_content_fallback(registry, provider_id, source_path)?
    } else {
        None
    };

    Ok(SessionDetail {
        messages,
        qa_pairs,
        raw_content,
    })
}

pub fn extract_qa_pairs(messages: &[SessionMessage]) -> Vec<QaPair> {
    let mut pairs = Vec::new();
    let mut pending_user_idx: Option<usize> = None;
    let mut pending_answer_idx: Option<usize> = None;

    for (i, message) in messages.iter().enumerate() {
        match message.role.to_lowercase().as_str() {
            "user" => {
                if let (Some(q), Some(a)) = (pending_user_idx, pending_answer_idx) {
                    pairs.push(QaPair {
                        question_idx: q,
                        answer_idx: a,
                    });
                }
                pending_user_idx = Some(i);
                pending_answer_idx = None;
            }
            "assistant" => {
                if pending_user_idx.is_some() {
                    pending_answer_idx = Some(i);
                }
            }
            _ => {}
        }
    }

    if let (Some(q), Some(a)) = (pending_user_idx, pending_answer_idx) {
        pairs.push(QaPair {
            question_idx: q,
            answer_idx: a,
        });
    }

    pairs
}
