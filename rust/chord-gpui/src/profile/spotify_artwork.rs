//! Fetches validated Spotify artwork using the existing bounded HTTPS transport.
#![forbid(unsafe_code)]

use std::{future::Future, sync::Arc, time::Duration};

use chord_core::rest::{
    routes::HttpMethod,
    transport::{HttpClientConfig, HttpRequest, send_with},
};
use gpui::*;

use crate::theme::DISCORD_THEME;

const ARTWORK_HOST: &str = "https://i.scdn.co";

pub(super) fn cover(url: Option<&str>, window: &mut Window, cx: &mut App) -> AnyElement {
    let image = url
        .and_then(|url| url.strip_prefix(ARTWORK_HOST))
        .and_then(|path| window.use_asset::<AssetLogger<SpotifyArtwork>>(&path.to_owned(), cx))
        .and_then(Result::ok);
    if let Some(image) = image {
        return img(image)
            .size(px(48.))
            .flex_shrink_0()
            .rounded(px(4.))
            .object_fit(ObjectFit::Cover)
            .into_any_element();
    }
    div()
        .size(px(48.))
        .flex_shrink_0()
        .rounded(px(4.))
        .bg(DISCORD_THEME.bg_active)
        .flex()
        .justify_center()
        .items_center()
        .text_color(DISCORD_THEME.text_muted)
        .child("♫")
        .into_any_element()
}

struct SpotifyArtwork;

impl Asset for SpotifyArtwork {
    type Source = String;
    type Output = Result<Arc<Image>, String>;

    fn load(
        path: Self::Source,
        _: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move { fetch_artwork(path).await }
    }
}

async fn fetch_artwork(path: String) -> Result<Arc<Image>, String> {
    // GPUI uses its own executor. Enter a bounded Tokio runtime for the shared HTTPS client.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let response = runtime.block_on(async {
        let config = HttpClientConfig {
            max_body_bytes: 2 * 1024 * 1024,
            ..Default::default()
        };
        let request = HttpRequest::new(HttpMethod::Get, path);
        tokio::time::timeout(
            Duration::from_secs(6),
            send_with(&config, ARTWORK_HOST, &request),
        )
        .await
        .map_err(|_| "Spotify artwork request timed out".to_owned())?
        .map_err(|error| format!("Spotify artwork transport failed: {error:?}"))
    })?;
    if response.status != 200 {
        return Err(format!("Spotify artwork HTTP {}", response.status));
    }
    let format = match image::guess_format(&response.body) {
        Ok(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
        Ok(image::ImageFormat::Png) => ImageFormat::Png,
        _ => return Err("Unsupported Spotify artwork format".into()),
    };
    Ok(Arc::new(Image::from_bytes(format, response.body)))
}
