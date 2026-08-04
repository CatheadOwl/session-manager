use super::providers::ProviderRegistry;
use super::types::{QaPair, SessionDetail, SessionHandle, SessionMessage};
use std::time::Instant;

#[allow(dead_code)]
#[deprecated(note = "use load_messages_for_handle instead")]
pub fn load_messages(
    registry: &ProviderRegistry,
    provider_id: &str,
    source_path: &str,
) -> Result<Vec<SessionMessage>, String> {
    let handle = SessionHandle {
        provider_id: provider_id.to_string(),
        session_id: String::new(),
        locator: super::types::SessionLocator::File {
            path: source_path.to_string(),
        },
    };
    load_messages_for_handle(registry, &handle)
}

pub fn load_messages_for_handle(
    registry: &ProviderRegistry,
    handle: &SessionHandle,
) -> Result<Vec<SessionMessage>, String> {
    let start = Instant::now();
    log::debug!(
        "message_load start provider={} session={} locator={} path={}",
        handle.provider_id,
        handle.session_id,
        handle.locator.detail_key_part(),
        handle.display_source_path()
    );

    let provider = match registry.get(&handle.provider_id) {
        Ok(provider) => provider,
        Err(err) => {
            log::warn!(
                "message_load error provider={} session={} path={} elapsed_ms={} error={}",
                handle.provider_id,
                handle.session_id,
                handle.display_source_path(),
                start.elapsed().as_millis(),
                err
            );
            return Err(err);
        }
    };
    let result = provider.load_messages_for_handle(handle);

    match &result {
        Ok(messages) => log::debug!(
            "message_load finish provider={} session={} message_count={} elapsed_ms={}",
            handle.provider_id,
            handle.session_id,
            messages.len(),
            start.elapsed().as_millis()
        ),
        Err(err) => log::warn!(
            "message_load error provider={} session={} path={} elapsed_ms={} error={}",
            handle.provider_id,
            handle.session_id,
            handle.display_source_path(),
            start.elapsed().as_millis(),
            err
        ),
    }

    result
}

fn load_raw_content_fallback_for_handle(
    registry: &ProviderRegistry,
    handle: &SessionHandle,
) -> Result<Option<String>, String> {
    registry
        .get(&handle.provider_id)?
        .load_raw_content_fallback_for_handle(handle)
}

#[allow(dead_code)]
#[deprecated(note = "use load_session_detail_for_handle instead")]
pub fn load_session_detail(
    registry: &ProviderRegistry,
    provider_id: &str,
    source_path: &str,
) -> Result<SessionDetail, String> {
    let handle = SessionHandle {
        provider_id: provider_id.to_string(),
        session_id: String::new(),
        locator: super::types::SessionLocator::File {
            path: source_path.to_string(),
        },
    };
    load_session_detail_for_handle(registry, &handle)
}

pub fn load_session_detail_for_handle(
    registry: &ProviderRegistry,
    handle: &SessionHandle,
) -> Result<SessionDetail, String> {
    let start = Instant::now();
    log::debug!(
        "detail_load start provider={} session={} locator={} path={}",
        handle.provider_id,
        handle.session_id,
        handle.locator.detail_key_part(),
        handle.display_source_path()
    );

    let messages = load_messages_for_handle(registry, handle)?;
    let qa_pairs = extract_qa_pairs(&messages);
    let raw_content = if messages.is_empty() {
        load_raw_content_fallback_for_handle(registry, handle)?
    } else {
        None
    };

    let detail = SessionDetail {
        messages,
        qa_pairs,
        raw_content,
    };

    log::debug!(
        "detail_load finish provider={} session={} message_count={} qa_pair_count={} raw_fallback={} elapsed_ms={}",
        handle.provider_id,
        handle.session_id,
        detail.messages.len(),
        detail.qa_pairs.len(),
        detail.raw_content.is_some(),
        start.elapsed().as_millis()
    );

    Ok(detail)
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
