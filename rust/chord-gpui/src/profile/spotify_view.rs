//! Rendering and actions for Spotify listening on the member profile.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
use std::time::Instant;

use gpui::{prelude::FluentBuilder, *};
use gpui_kit::component::StyledExt;

use crate::{
    dispatch::profile::{spotify_mpris::format_mm_ss, spotify_widget::*},
    state::AppState,
    theme::DISCORD_THEME,
    ui::AppLayout,
};

#[cfg(target_os = "linux")]
use gpui_component::button::Button;

#[cfg(target_os = "linux")]
#[path = "spotify_player.rs"]
mod spotify_player;

#[path = "spotify_artwork.rs"]
mod spotify_artwork;

pub(super) fn render(
    state: &AppState,
    host_id: &str,
    widget: SpotifyProfileWidget,
    window: &mut Window,
    cx: &mut Context<AppLayout>,
) -> Div {
    let advancing = widget.is_advancing();
    if advancing {
        window.request_animation_frame();
    }
    let accent = rgb(u32::from_str_radix(&SPOTIFY_GREEN_HEX[1..], 16).unwrap_or_default());
    let cover = spotify_artwork::cover(widget.track.artwork_url.as_deref(), window, cx);
    div()
        .v_flex()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(6.))
        .bg(DISCORD_THEME.bg_primary)
        .border_1()
        .border_color(DISCORD_THEME.border_subtle)
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::BOLD)
                .text_color(accent)
                .child("Listening on Spotify"),
        )
        .child(
            div().flex().items_center().gap(px(8.)).child(cover).child(
                div()
                    .v_flex()
                    .min_w(px(0.))
                    .flex_1()
                    .child(track_title(&widget, window))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(DISCORD_THEME.text_muted)
                            .truncate()
                            .child(widget.track.artist.clone()),
                    ),
            ),
        )
        .when(widget.duration_ms() > 0, |element| {
            element.child(timeline(&widget, accent))
        })
        .children(listen_along_action(state, host_id, &widget, cx))
}

fn track_title(widget: &SpotifyProfileWidget, window: &Window) -> Div {
    // The profile has a fixed 320px width. Reserve space for art and both sets of padding.
    let viewport = px(202.);
    let font_size = px(13.);
    let title: SharedString = widget
        .track
        .title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .into();
    let run = TextRun {
        len: title.len(),
        font: window.text_style().font(),
        color: DISCORD_THEME.text_header,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let width = window
        .text_system()
        .shape_line(title.clone(), font_size, &[run], None)
        .width();
    let overflow = (width - viewport).max(px(0.));
    let offset = if overflow > px(0.) && widget.is_advancing() {
        let seconds_to_scroll = f32::from(overflow) / 28.;
        let phase = (widget.elapsed_ms() as f32 / 1000.) % (2. * seconds_to_scroll + 2.);
        px(((phase - 1.).min(2. * seconds_to_scroll + 1. - phase) * 28.).max(0.))
    } else {
        px(0.)
    };
    div()
        .relative()
        .w(viewport)
        .max_w_full()
        .h(px(20.))
        .overflow_hidden()
        .text_size(font_size)
        .text_color(DISCORD_THEME.text_header)
        .child(
            div()
                .absolute()
                .left(-offset)
                .w((width + px(2.)).max(viewport))
                .whitespace_nowrap()
                .child(title),
        )
}

fn timeline(widget: &SpotifyProfileWidget, accent: Rgba) -> Div {
    div()
        .v_flex()
        .gap(px(4.))
        .child(
            div()
                .w_full()
                .h(px(4.))
                .rounded(px(2.))
                .overflow_hidden()
                .bg(DISCORD_THEME.bg_active)
                .child(
                    div()
                        .h_full()
                        .w(relative(widget.progress_ratio() as f32))
                        .bg(accent),
                ),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(px(11.))
                .text_color(DISCORD_THEME.text_muted)
                .child(format_mm_ss(widget.elapsed_ms()))
                .child(format_mm_ss(widget.duration_ms())),
        )
}

fn listen_along_action(
    state: &AppState,
    host_id: &str,
    widget: &SpotifyProfileWidget,
    cx: &mut Context<AppLayout>,
) -> Option<AnyElement> {
    if state.current_user.id.is_empty() || state.current_user.id == host_id {
        return None;
    }
    let command = widget.listen_along_command()?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (command, cx);
        Some(status_text("Listen Along is unavailable on this desktop.").into_any_element())
    }
    #[cfg(target_os = "linux")]
    {
        let active = state.spotify_listen_along.as_ref();
        if active.is_some_and(|action| action.result.is_none()) {
            return Some(status_text("Starting Spotify…").into_any_element());
        }
        let feedback = active
            .filter(|action| action.host_id == host_id && action.track_id == command.track_id)
            .and_then(|action| action.result.as_ref())
            .map(|result| match result {
                Ok(()) => "Playback synchronized.".to_owned(),
                Err(error) => error.clone(),
            });

        let host_id = host_id.to_owned();
        Some(
            div()
                .v_flex()
                .gap(px(4.))
                .child(
                    Button::new("spotify-listen-along")
                        .label("Listen Along")
                        .on_click(cx.listener(move |layout, _, _, cx| {
                            start_listen_along(layout, &host_id, cx);
                        })),
                )
                .children(feedback.map(status_text))
                .into_any_element(),
        )
    }
}

fn status_text(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(DISCORD_THEME.text_muted)
        .child(text.into())
}

#[cfg(target_os = "linux")]
fn start_listen_along(layout: &mut AppLayout, host_id: &str, cx: &mut Context<AppLayout>) {
    use crate::dispatch::profile::presence::ListenAlongStatus;

    let state = &mut layout.state;
    if state
        .spotify_listen_along
        .as_ref()
        .is_some_and(|action| action.result.is_none())
        || state.current_user.id.is_empty()
        || state.current_user.id == host_id
        || !state
            .selected_member_profile
            .as_ref()
            .is_some_and(|member| member.id == host_id)
    {
        return;
    }
    let requested_at = Instant::now();
    let Some(command) = state
        .spotify_profiles
        .get(host_id)
        .and_then(|snapshot| snapshot.widget_at(requested_at).listen_along_command())
    else {
        return;
    };
    let (cancellation, cancelled) = async_channel::bounded(1);
    state.spotify_listen_along = Some(ListenAlongStatus {
        host_id: host_id.to_owned(),
        track_id: command.track_id.clone(),
        cancellation: cancellation.clone(),
        result: None,
    });
    cx.notify();

    let background = cx.background_executor().clone();
    let task = background.spawn(spotify_player::listen_along(
        command,
        requested_at,
        background.clone(),
        cancelled,
    ));
    cx.spawn(async move |layout, cx| {
        let result = task.await;
        let _ = layout.update(cx, |layout, cx| {
            if let Some(action) = &mut layout.state.spotify_listen_along
                && action.cancellation.same_channel(&cancellation)
            {
                action.result = Some(result);
                cx.notify();
            }
        });
    })
    .detach();
}
