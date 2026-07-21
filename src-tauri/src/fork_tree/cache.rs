use std::path::Path;

use super::types::{ForkTreeCache, CACHE_VERSION};

pub(crate) fn load_cache(path: &Path) -> ForkTreeCache {
    if !path.exists() {
        return ForkTreeCache {
            version: CACHE_VERSION,
            files: Vec::new(),
        };
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return ForkTreeCache {
                version: CACHE_VERSION,
                files: Vec::new(),
            };
        }
    };

    match serde_json::from_str::<ForkTreeCache>(&content) {
        Ok(cache) => {
            if cache.version != CACHE_VERSION {
                ForkTreeCache {
                    version: CACHE_VERSION,
                    files: Vec::new(),
                }
            } else {
                cache
            }
        }
        Err(_) => ForkTreeCache {
            version: CACHE_VERSION,
            files: Vec::new(),
        },
    }
}

pub(crate) fn save_cache(path: &Path, cache: &ForkTreeCache) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create cache directory: {e}"))?;
    }

    let json = serde_json::to_string_pretty(cache)
        .map_err(|e| format!("Failed to serialize fork tree cache: {e}"))?;
    std::fs::write(path, &json).map_err(|e| format!("Failed to write fork tree cache: {e}"))?;
    Ok(())
}
