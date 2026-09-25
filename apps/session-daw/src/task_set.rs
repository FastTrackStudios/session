//! A set Task keeps, reached: joined by its live link, and tried again
//! until Task answers — a page and the app join the one way.

/// Try `attempt` until it works — or `limit` times — waiting between
/// tries from a second, doubling to fifteen; `failed` hears why each one
/// did not, and when the next is.
///
/// # Errors
///
/// The last try's, once `limit` tries have failed.
pub async fn until_ok<T, Fut>(
    limit: Option<u32>,
    mut attempt: impl FnMut() -> Fut,
    mut failed: impl FnMut(String),
) -> eyre::Result<T>
where
    Fut: std::future::Future<Output = eyre::Result<T>>,
{
    let mut wait = std::time::Duration::from_secs(1);
    let mut tries = 0u32;
    loop {
        match attempt().await {
            Ok(done) => return Ok(done),
            Err(e) => {
                tries += 1;
                if limit.is_some_and(|limit| tries >= limit) {
                    return Err(e);
                }
                tracing::debug!(error = %e, tries, "task: trying again");
                failed(format!("{e} — trying again in {} s", wait.as_secs()));
                architect::platform::sleep(wait).await;
                wait = (wait * 2).min(std::time::Duration::from_secs(15));
            }
        }
    }
}

/// Join the set a live link opens, on the Task at `url` (its vox lane —
/// [`crate::collab::TaskSet::parse`] makes it from the link).
///
/// # Errors
///
/// Task is not answering, or would not let the set be joined.
pub async fn join(url: &str) -> eyre::Result<live_proto::LiveSet> {
    use live_proto::LiveSessionsClient;
    let lane: LiveSessionsClient = task_dial::establish_at(url, None)
        .await
        .map_err(|e| eyre::eyre!("Task is not answering ({e})"))?;
    lane.join(String::new())
        .await
        .map_err(|e| eyre::eyre!("the set could not be joined ({e:?})"))
}
