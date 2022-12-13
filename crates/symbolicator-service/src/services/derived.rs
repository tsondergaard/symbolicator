use std::sync::Arc;

use crate::cache::{cache_entry_as_cache_status, CacheEntry, CacheError};
use crate::services::objects::{FoundObject, ObjectError, ObjectMetaHandle};
use crate::types::{AllObjectCandidates, CandidateStatus, ObjectFeatures, ObjectUseInfo};

/// This is the result of fetching a derived cache file.
///
/// This has the requested [`CacheEntry`], as well as [`AllObjectCandidates`] that were considered
/// and the [`ObjectFeatures`] of the primarily used object file.
#[derive(Clone, Debug)]
pub struct DerivedCache<T> {
    pub cache: CacheEntry<T>,
    pub candidates: AllObjectCandidates,
    pub features: ObjectFeatures,
}

/// Derives a [`DerivedCache`] from the provided object handle and derive function.
///
/// This function is mainly a wrapper that simplifies error handling and propagation of
/// [`AllObjectCandidates`] and [`ObjectFeatures`].
/// The [`CandidateStatus`] is responsible for telling which status to set on the found candidate.
pub async fn derive_from_object_handle<T, Derive, DeriveErr, Fut>(
    found_object: Result<FoundObject, ObjectError>,
    candidate_status: CandidateStatus,
    derive: Derive,
) -> DerivedCache<T>
where
    T: Clone,
    Derive: FnOnce(Arc<ObjectMetaHandle>) -> Fut,
    Fut: std::future::Future<Output = Result<Arc<CacheEntry<T>>, Arc<DeriveErr>>>,
    for<'a> &'a DeriveErr: Into<CacheError>,
{
    let (meta, mut candidates) = match found_object {
        Ok(FoundObject { meta, candidates }) => (meta, candidates),
        Err(e) => {
            // NOTE: the only error that can happen here is if the Sentry downloader `list_files`
            // fails, which we can consider a download error
            let dynerr: &dyn std::error::Error = &e; // tracing expects a `&dyn Error`
            tracing::error!(error = dynerr, "Error finding object candidates");
            return DerivedCache {
                cache: Err(CacheError::DownloadError(e.to_string())),
                candidates: Default::default(),
                features: Default::default(),
            };
        }
    };

    // No handle => NotFound
    let Some(handle) = meta else {
        return DerivedCache {
            cache: Err(CacheError::NotFound),
            candidates,
            features: ObjectFeatures::default(),
        }
    };

    // Fetch cache file from handle
    let derived_cache = derive(Arc::clone(&handle)).await;

    match derived_cache {
        Ok(cache) => {
            candidates.set_status(
                candidate_status,
                handle.source_id(),
                &handle.uri(),
                ObjectUseInfo::from_derived_status(
                    &cache_entry_as_cache_status(&cache),
                    handle.status(),
                ),
            );

            DerivedCache {
                cache: Arc::try_unwrap(cache).unwrap_or_else(|arc| (*arc).clone()),
                candidates,
                features: handle.features(),
            }
        }

        // TODO: Internal Err fetching from cache, this should really be merged with `Ok`, as this is
        // essentially a `Result<Result>` right now.
        // `Cacher::compute_memoized` should just convert all the internal caching-related errors to
        // `CacheError` instead of taking a detour through a type parameter.
        Err(e) => DerivedCache {
            cache: Err(e.as_ref().into()),
            candidates,
            features: ObjectFeatures::default(),
        },
    }
}
