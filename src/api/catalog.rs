use super::*;

#[get(
    "/api/v1/search?query&filter",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn search_catalog(query: String, filter: String) -> Result<SearchResults> {
    let mut answer = state
        .search_catalog(&owner.0, &query, &filter)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    state
        .overlay_viewer(&owner.0, &mut answer.videos)
        .await
        .map_err(server_error)?;
    Ok(answer)
}

#[get(
    "/api/v1/search/page?query&filter&next_page",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn search_catalog_page(
    query: String,
    filter: String,
    next_page: String,
) -> Result<SearchResults> {
    let mut answer = state
        .search_page(&owner.0, &query, &filter, &next_page)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    state
        .overlay_viewer(&owner.0, &mut answer.videos)
        .await
        .map_err(server_error)?;
    Ok(answer)
}

#[get(
    "/api/v1/videos/{video_id}",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_video_details(video_id: String) -> Result<VideoDetails> {
    Ok(state
        .video_details(&owner.0, &video_id)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?)
}

#[get(
    "/api/v1/videos/{video_id}/comments?next_page",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>
)]
pub async fn get_comments_page(video_id: String, next_page: String) -> Result<CommentsPage> {
    Ok(state
        .comments_page(&video_id, &next_page)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?)
}

#[get(
    "/api/v1/channels/{channel_id}?order",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_channel_details(channel_id: String, order: String) -> Result<ChannelDetails> {
    Ok(state
        .channel_details(&owner.0, &channel_id, &order)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?)
}

// A POST: the continuation token is several KB, which a query string
// carries badly (proxies refuse long request lines).
#[post(
    "/api/v1/channels/{channel_id}/media",
    state: dioxus::fullstack::extract::State<crate::server::AppServerState>,
    owner: crate::auth::Owner
)]
pub async fn get_channel_media_page(
    channel_id: String,
    tab: String,
    next_page: String,
) -> Result<ChannelMediaPage> {
    let tab = match tab.as_str() {
        "shorts" => crate::models::ChannelMediaTab::Shorts,
        "live" => crate::models::ChannelMediaTab::Live,
        _ => crate::models::ChannelMediaTab::Videos,
    };
    let mut answer = state
        .channel_media_page(&owner.0, &channel_id, tab, &next_page)
        .await
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    state
        .overlay_viewer(&owner.0, &mut answer.videos)
        .await
        .map_err(server_error)?;
    Ok(answer)
}
