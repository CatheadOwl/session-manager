use std::collections::HashMap;

use super::types::{CachedFileData, TreeNodeData};

/// Extract the session_id component from a session_key of the form
/// `"provider_id:session_id:source_path"`.
fn session_id_from_key(key: &str) -> &str {
    key.splitn(3, ':').nth(1).unwrap_or("")
}

/// Check whether `ancestor` is (directly or transitively) an ancestor of
/// `descendant` in the parent array. Used to prevent cycles when mixing
/// explicit `forked_from_id` references with heuristic matching.
fn is_ancestor_of(ancestor: usize, descendant: usize, parent: &[Option<usize>]) -> bool {
    let mut current = descendant;
    while let Some(p) = parent[current] {
        if p == ancestor {
            return true;
        }
        current = p;
    }
    false
}

/// Build a tree from a list of file data.
///
/// Processing is per-provider, with three independent paths:
///
/// **Path A — Forked-from-ID providers** (e.g. Codex): Sessions with a
/// `forked_from_id` are resolved against the session-key lookup table.
/// No heuristic parent matching is applied — all parent relationships are
/// authoritative. `fork_at` prefers `uuid_chain` when both sides have it,
/// falling back to `hash_chain`.
///
/// **Path B — UUID chain LCP** (Claude with UUIDs): The same LCP algorithm
/// as Path C, but compares `uuid_chain` instead of `hash_chain`. This is a
/// stronger matching signal because `claude --resume` preserves the parent's
/// original message UUIDs in the child session.
///
/// **Path C — Hash chain LCP** (original heuristic): Sessions are sorted by
/// hash-chain length (shortest first). Each session finds the already-processed
/// one with the longest common prefix of hash-chain entries; that becomes its
/// parent. This correctly handles `claude --resume` where the child inherits
/// the parent's full message history as a prefix. Used when no uuid data is
/// available. No `forked_from_id` awareness.
pub(crate) fn build_tree(files: &[CachedFileData]) -> Vec<TreeNodeData> {
    let n = files.len();
    if n == 0 {
        return Vec::new();
    }

    // Group indices by provider
    let mut provider_map: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, file) in files.iter().enumerate() {
        let provider = file.session_key.splitn(3, ':').next().unwrap_or("");
        provider_map.entry(provider).or_default().push(i);
    }

    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut fork_at: Vec<u32> = vec![0; n];

    for group in provider_map.into_values() {
        let uses_fkid = group.iter().any(|&i| files[i].forked_from_id.is_some());

        if uses_fkid {
            // ── Path A: forked_from_id ───────────────────────────────────
            // Build per-provider lookup so forked_from_id never cross-matches
            // a different provider's session_id.
            let mut session_id_to_idx: HashMap<&str, usize> = HashMap::new();
            for &i in &group {
                let sid = session_id_from_key(&files[i].session_key);
                if !sid.is_empty() {
                    session_id_to_idx.insert(sid, i);
                }
            }

            for &i in &group {
                if let Some(ref fkid) = files[i].forked_from_id {
                    if let Some(&parent_idx) = session_id_to_idx.get(fkid.as_str()) {
                        if parent_idx != i && !is_ancestor_of(i, parent_idx, &parent) {
                            parent[i] = Some(parent_idx);
                            // Prefer uuid_chain for fork_at if both sides have it
                            fork_at[i] = if !files[i].uuid_chain.is_empty()
                                && !files[parent_idx].uuid_chain.is_empty()
                            {
                                common_prefix_len(
                                    &files[i].uuid_chain,
                                    &files[parent_idx].uuid_chain,
                                ) as u32
                            } else {
                                common_prefix_len(
                                    &files[i].hash_chain,
                                    &files[parent_idx].hash_chain,
                                ) as u32
                            };
                        }
                    }
                }
            }
        } else {
            let uses_uuid = group.iter().any(|&i| !files[i].uuid_chain.is_empty());
            if uses_uuid {
                // ── Path B: UUID chain LCP ───────────────────────────────
                // Uses the same LCP algorithm as Path C, but compares
                // uuid_chain instead of hash_chain for stronger cross-session
                // matching (Claude --resume preserves original uuids).
                let mut sorted: Vec<usize> = group;
                sorted.sort_by(|&a, &b| {
                    files[a]
                        .uuid_chain
                        .len()
                        .cmp(&files[b].uuid_chain.len())
                        .then_with(|| files[a].session_key.cmp(&files[b].session_key))
                });

                let mut processed: Vec<usize> = Vec::new();
                for &i in &sorted {
                    let mut best_parent: Option<usize> = None;
                    let mut best_prefix: usize = 0;
                    for &j in &processed {
                        let prefix = common_prefix_len(&files[i].uuid_chain, &files[j].uuid_chain);
                        if prefix > best_prefix {
                            best_prefix = prefix;
                            best_parent = Some(j);
                        }
                    }
                    parent[i] = best_parent;
                    fork_at[i] = best_prefix as u32;
                    processed.push(i);
                }
            } else {
                // ── Path C: hash chain LCP (original heuristic) ─────────
                let mut sorted: Vec<usize> = group;
                sorted.sort_by(|&a, &b| {
                    files[a]
                        .hash_chain
                        .len()
                        .cmp(&files[b].hash_chain.len())
                        .then_with(|| files[a].session_key.cmp(&files[b].session_key))
                });

                let mut processed: Vec<usize> = Vec::new();
                for &i in &sorted {
                    let mut best_parent: Option<usize> = None;
                    let mut best_prefix: usize = 0;
                    for &j in &processed {
                        let prefix = common_prefix_len(&files[i].hash_chain, &files[j].hash_chain);
                        if prefix > best_prefix {
                            best_prefix = prefix;
                            best_parent = Some(j);
                        }
                    }
                    parent[i] = best_parent;
                    fork_at[i] = best_prefix as u32;
                    processed.push(i);
                }
            }
        }
    }

    // Compute depth by walking parent chain
    let mut depth: Vec<u32> = vec![0; n];
    for i in 0..n {
        if parent[i].is_some() {
            let mut d = 0u32;
            let mut current = i;
            while let Some(p) = parent[current] {
                d += 1;
                current = p;
            }
            depth[i] = d;
        }
    }

    // Allocate all tree nodes
    let nodes: Vec<TreeNodeData> = (0..n)
        .map(|i| {
            let chain_idx = fork_at[i] as usize;
            // fork_user_text is looked up in chain-space (user_texts is parallel to hash_chain)
            let fork_user_text = if depth[i] > 0 {
                files[i].user_texts.get(chain_idx).cloned()
            } else {
                None
            };
            // Map chain-space index back to original (unfiltered) event index for UI jump.
            // Fallback: if kept_indices is unavailable (legacy), use chain index directly.
            let forked_at_user = if depth[i] > 0 && !files[i].kept_indices.is_empty() {
                files[i]
                    .kept_indices
                    .get(chain_idx)
                    .copied()
                    .unwrap_or_else(|| {
                        // Edge: fork at end of chain (session is strict prefix of parent)
                        files[i].kept_indices.last().map(|k| k + 1).unwrap_or(chain_idx)
                    }) as u32
            } else {
                fork_at[i]
            };
            TreeNodeData {
                session_key: files[i].session_key.clone(),
                title: files[i].title.clone(),
                summary: files[i].summary.clone(),
                last_active_at: files[i].last_active_at,
                project_dir: files[i].project_dir.clone(),
                user_hash_chain: files[i].hash_chain.clone(),
                depth: depth[i],
                forked_at_user,
                fork_user_text,
                children: Vec::new(),
            }
        })
        .collect();

    // Build children lists and identify roots
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();

    for i in 0..n {
        match parent[i] {
            Some(p) => children_of[p].push(i),
            None => roots.push(i),
        }
    }

    // Recursively build tree, consuming each node exactly once
    fn build_subtree(
        idx: usize,
        nodes: &mut Vec<Option<TreeNodeData>>,
        children_of: &[Vec<usize>],
    ) -> TreeNodeData {
        let mut node = nodes[idx].take().expect("node already consumed");
        for &child_idx in &children_of[idx] {
            node.children
                .push(build_subtree(child_idx, nodes, children_of));
        }
        // Stable sort within each parent
        node.children.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.session_key.cmp(&b.session_key))
        });
        node
    }

    let mut opt_nodes: Vec<Option<TreeNodeData>> = nodes.into_iter().map(Some).collect();
    let mut result: Vec<TreeNodeData> = roots
        .iter()
        .map(|&i| build_subtree(i, &mut opt_nodes, &children_of))
        .collect();

    // Sort roots
    result.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.session_key.cmp(&b.session_key))
    });

    result
}

fn common_prefix_len(a: &[String], b: &[String]) -> usize {
    let max_len = a.len().min(b.len());
    let mut count = 0;
    for i in 0..max_len {
        if a[i] == b[i] {
            count += 1;
        } else {
            break;
        }
    }
    count
}
