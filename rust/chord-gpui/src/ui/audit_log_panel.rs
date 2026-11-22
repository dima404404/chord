//! Native audit panel. Network replies belong to one guild and account context.
use crate::guild::audit_log::{
    self as model, AuditChangeItem, AuditChangeValue, AuditLogEntryRecord, GuildAuditLogQuery,
};
use crate::guild::audit_log_view::{
    self as view, AuditLoadState, AuditLogRowViewSpec, AuditLogViewer, AuditRowInteractiveState,
};
use crate::net::{DiscordRestClient, NetworkConfig, RestTransport};
use crate::state::AppState;
use crate::theme::DISCORD_THEME;
use crate::ui::AppLayout;
use chord_core::rest::client::{ApiClient, ApiConfig};
use chord_core::rest::transport::HttpResponse;
use gpui::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Default)]
pub struct AuditLogPanel {
    pub viewer: AuditLogViewer,
    pub is_open: bool,
    guild_id: Option<String>,
    account_id: Option<String>,
    receiver: Option<Receiver<Result<AuditReply, String>>>,
    action_menu_open: bool,
    user_menu_open: bool,
}

struct AuditReply {
    status: u16,
    entries: Vec<AuditLogEntryRecord>,
    retry_after_ms: Option<u64>,
}

impl AuditLogPanel {
    pub fn open(&mut self, state: &AppState) {
        self.close();
        self.is_open = true;
        self.guild_id = state.selected_guild_id.clone();
        self.account_id = Some(state.current_user.id.clone());
        let Some(guild_id) = self.guild_id.clone() else {
            self.viewer
                .set_error("Select a server to view its audit log.");
            return;
        };
        let Some(token) = state
            .active_token
            .clone()
            .filter(|token| !token.trim().is_empty())
        else {
            self.viewer
                .set_error("An authenticated account is required to view the audit log.");
            return;
        };
        self.viewer.begin_loading();
        let account_id = state.current_user.id.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        self.receiver = Some(receiver);
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("Audit runtime initialization failed: {error}"))
                .and_then(|runtime| runtime.block_on(load_audit(&guild_id, &account_id, token)));
            let _ = sender.send(result);
        });
    }

    pub fn close(&mut self) {
        self.receiver = None;
        self.viewer.clear();
        self.is_open = false;
        self.guild_id = None;
        self.account_id = None;
        self.action_menu_open = false;
        self.user_menu_open = false;
    }

    pub fn matches_context(&self, state: &AppState) -> bool {
        self.guild_id == state.selected_guild_id
            && self.account_id.as_deref() == Some(state.current_user.id.as_str())
    }

    fn retain_context(&mut self, guild: Option<&str>, account: &str) -> bool {
        if self.guild_id.as_deref() == guild && self.account_id.as_deref() == Some(account) {
            return true;
        }
        self.close();
        false
    }

    pub fn poll(&mut self, state: &AppState) -> bool {
        if !self.is_open {
            return false;
        }
        if !self.retain_context(state.selected_guild_id.as_deref(), &state.current_user.id) {
            return true;
        }
        let reply = match self.receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(reply)) => reply,
            Some(Err(TryRecvError::Disconnected)) => {
                Err("Audit worker ended without a response.".to_string())
            }
            _ => return false,
        };
        self.receiver = None;
        match reply {
            Ok(reply) => {
                if reply.status == 200 {
                    if let Err(error) = self.viewer.replace_entries(reply.entries) {
                        self.viewer.set_error(&error.to_string());
                    }
                } else {
                    self.viewer
                        .apply_http_status(reply.status, reply.retry_after_ms);
                }
            }
            Err(error) => self.viewer.set_error(&error),
        }
        true
    }

    pub fn render(&self, cx: &mut Context<AppLayout>) -> AnyElement {
        let rows = self.viewer.rows();
        let mut body = div()
            .id("audit-log-rows")
            .flex()
            .flex_col()
            .gap(px(8.))
            .flex_1()
            .overflow_y_scroll();
        if self.viewer.load_state != AuditLoadState::Ready || rows.is_empty() {
            body = body.child(self.status_text());
        }
        for row in rows {
            body = body.child(render_row(row, cx));
        }
        div()
            .absolute()
            .inset_0()
            .bg(rgba(0x00000088))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("audit-log-panel")
                    .w(px(760.))
                    .h(px(600.))
                    .max_w_full()
                    .max_h_full()
                    .p(px(view::AUDIT_LOG_VIEWER_PADDING_PX as f32))
                    .bg(color(view::BACKGROUND))
                    .text_color(color(view::TEXT))
                    .border_1()
                    .border_color(color(view::BORDER))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(self.header(cx))
                    .child(self.filters(cx))
                    .child(body),
            )
            .into_any_element()
    }

    fn status_text(&self) -> String {
        if let Some(error) = &self.viewer.error_message {
            return error.clone();
        }
        match self.viewer.load_state {
            AuditLoadState::Idle => "Audit log has not been loaded.".to_string(),
            AuditLoadState::Loading => "Loading audit log…".to_string(),
            AuditLoadState::Deferred => "Audit log is still being prepared (HTTP 202).".to_string(),
            AuditLoadState::Forbidden => {
                "Audit log access denied. VIEW_AUDIT_LOG is required.".to_string()
            }
            AuditLoadState::Failed(status) => format!(
                "Audit log request failed (HTTP {status}). Retry after: {:?} ms.",
                self.viewer.retry_after_ms
            ),
            AuditLoadState::Ready => "No audit log entries match these filters.".to_string(),
        }
    }

    fn header(&self, cx: &mut Context<AppLayout>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(20.))
                    .child(view::AUDIT_LOG_VIEWER_CONTAINER),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("refresh-audit-log")
                            .cursor_pointer()
                            .child("Обновить")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    if this.audit_log_panel.viewer.load_state
                                        != AuditLoadState::Loading
                                    {
                                        this.audit_log_panel.open(&this.state);
                                        cx.notify();
                                    }
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id("close-audit-log")
                            .cursor_pointer()
                            .child("Закрыть")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.audit_log_panel.close();
                                    cx.notify();
                                }),
                            ),
                    ),
            )
    }

    fn filters(&self, cx: &mut Context<AppLayout>) -> impl IntoElement {
        div()
            .flex()
            .gap(px(8.))
            .child(self.action_filter(cx))
            .child(self.user_filter(cx))
    }

    fn action_filter(&self, cx: &mut Context<AppLayout>) -> impl IntoElement {
        let label = self
            .viewer
            .action_options()
            .into_iter()
            .find(|option| option.action_type == self.viewer.selected_action_filter)
            .map(|option| option.label)
            .unwrap_or_else(|| "Все действия".to_string());
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .id("audit-action-filter")
                    .cursor_pointer()
                    .p(px(8.))
                    .bg(color(view::SURFACE))
                    .child(format!("Фильтр по действию: {label} ▾"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.audit_log_panel.action_menu_open =
                                !this.audit_log_panel.action_menu_open;
                            this.audit_log_panel.user_menu_open = false;
                            cx.notify();
                        }),
                    ),
            )
            .children(self.action_menu_open.then(|| self.action_options(cx)))
    }

    fn user_filter(&self, cx: &mut Context<AppLayout>) -> impl IntoElement {
        let label = self
            .viewer
            .user_options()
            .into_iter()
            .find(|option| option.user_id == self.viewer.selected_user_filter)
            .map(|option| option.label)
            .unwrap_or_else(|| "Все пользователи".to_string());
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .id("audit-user-filter")
                    .cursor_pointer()
                    .p(px(8.))
                    .bg(color(view::SURFACE))
                    .child(format!("Фильтр по пользователю: {label} ▾"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.audit_log_panel.user_menu_open =
                                !this.audit_log_panel.user_menu_open;
                            this.audit_log_panel.action_menu_open = false;
                            cx.notify();
                        }),
                    ),
            )
            .children(self.user_menu_open.then(|| self.user_options(cx)))
    }

    fn action_options(&self, cx: &mut Context<AppLayout>) -> impl IntoElement {
        let mut menu = div()
            .id("audit-action-menu")
            .max_h(px(180.))
            .overflow_y_scroll()
            .bg(color(view::SURFACE))
            .flex()
            .flex_col();
        for option in self.viewer.action_options() {
            let action = option.action_type;
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("audit-action-{action:?}")))
                    .p(px(8.))
                    .cursor_pointer()
                    .hover(|style| style.bg(color(view::BORDER)))
                    .child(option.label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.audit_log_panel.viewer.filter_by_action(action);
                            this.audit_log_panel.action_menu_open = false;
                            cx.notify();
                        }),
                    ),
            );
        }
        menu
    }

    fn user_options(&self, cx: &mut Context<AppLayout>) -> impl IntoElement {
        let mut menu = div()
            .id("audit-user-menu")
            .max_h(px(180.))
            .overflow_y_scroll()
            .bg(color(view::SURFACE))
            .flex()
            .flex_col();
        for option in self.viewer.user_options() {
            let id = option.user_id;
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("audit-user-{id:?}")))
                    .p(px(8.))
                    .cursor_pointer()
                    .hover(|style| style.bg(color(view::BORDER)))
                    .child(option.label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Err(error) =
                                this.audit_log_panel.viewer.filter_by_user(id.clone())
                            {
                                this.audit_log_panel.viewer.set_error(&error.to_string());
                            }
                            this.audit_log_panel.user_menu_open = false;
                            cx.notify();
                        }),
                    ),
            );
        }
        menu
    }
}

fn render_row(row: AuditLogRowViewSpec, cx: &mut Context<AppLayout>) -> impl IntoElement {
    let id = row.entry_id;
    let expanded = row.state == AuditRowInteractiveState::ExpandedDiff;
    let mut details = div().flex().flex_col().gap(px(4.));
    if expanded {
        for change in row.details {
            details = details.child(div().child(change));
        }
    }
    div()
        .id(SharedString::from(format!("audit-row-{id}")))
        .p(px(8.))
        .border_1()
        .border_color(color(if row.focused {
            view::ACCENT
        } else {
            view::BORDER
        }))
        .bg(color(view::SURFACE))
        .cursor_pointer()
        .hover(|style| style.bg(color(view::BORDER)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.audit_log_panel.viewer.focused_entry_id = Some(id);
                this.audit_log_panel.viewer.toggle_row_expanded(id);
                cx.notify();
            }),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    div()
                        .text_color(color(row.icon.badge_color))
                        .child(row.icon.glyph),
                )
                .child(div().child(row.event_description_ru)),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(DISCORD_THEME.text_muted)
                .child(row.timestamp_iso),
        )
        .children(expanded.then_some(details))
}

fn color(raw: &str) -> Hsla {
    u32::from_str_radix(raw.trim_start_matches('#'), 16)
        .map(|value| rgb(value).into())
        .unwrap_or(DISCORD_THEME.text_header)
}

async fn load_audit(guild: &str, account: &str, token: String) -> Result<AuditReply, String> {
    model::validate_snowflake(guild).map_err(|error| error.to_string())?;
    model::validate_snowflake(account).map_err(|error| error.to_string())?;
    let config = NetworkConfig {
        token: Some(token),
        ..Default::default()
    };
    let auth = DiscordRestClient::new(config).build_auth()?;
    let client = ApiClient::new(ApiConfig {
        auth,
        ..Default::default()
    })
    .map_err(|_| "Could not initialize the authenticated audit client.".to_string())?;
    if let Some(denied) = check_remote_access(&client, guild, account).await? {
        return Ok(denied);
    }
    let request = model::build_guild_audit_log_request(&GuildAuditLogQuery {
        guild_id: guild.to_string(),
        limit: Some(100),
        ..Default::default()
    })
    .map_err(|error| error.to_string())?;
    let response = client.send_request("GET", &request.path, &[], None).await?;
    let mut reply = status_reply(&response);
    if response.status == 200 {
        reply.entries = parse_audit_response(&response.body)?;
    }
    Ok(reply)
}

async fn check_remote_access(
    client: &ApiClient,
    guild: &str,
    account: &str,
) -> Result<Option<AuditReply>, String> {
    let guild_data = match fetch_json(client, &format!("/guilds/{guild}")).await? {
        Ok(data) => data,
        Err(reply) => return Ok(Some(reply)),
    };
    let owner = required_id(&guild_data, "owner_id")?;
    if owner == account {
        return Ok(None);
    }
    let member = match fetch_json(client, &format!("/guilds/{guild}/members/{account}")).await? {
        Ok(data) => data,
        Err(reply) => return Ok(Some(reply)),
    };
    let number = |id: &str| model::validate_snowflake(id).map_err(|error| error.to_string());
    let bits = guild_permissions(
        number(guild)?,
        number(&owner)?,
        number(account)?,
        &guild_data,
        &member,
    )?;
    if model::check_audit_log_view_permission(bits, false, false).is_err() {
        return Ok(Some(AuditReply {
            status: 403,
            entries: Vec::new(),
            retry_after_ms: None,
        }));
    }
    Ok(None)
}

async fn fetch_json(client: &ApiClient, path: &str) -> Result<Result<Value, AuditReply>, String> {
    let response = client.send_request("GET", path, &[], None).await?;
    if response.status != 200 {
        return Ok(Err(status_reply(&response)));
    }
    serde_json::from_slice(&response.body)
        .map(Ok)
        .map_err(|_| "Invalid JSON while checking audit permissions.".to_string())
}

fn status_reply(response: &HttpResponse) -> AuditReply {
    let retry_after_ms = response
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.)
        .map(|seconds| std::time::Duration::from_secs_f64(seconds.min(86_400.)).as_millis())
        .and_then(|millis| u64::try_from(millis).ok());
    AuditReply {
        status: response.status,
        entries: Vec::new(),
        retry_after_ms,
    }
}

fn guild_permissions(
    guild: u64,
    owner: u64,
    user: u64,
    data: &Value,
    member: &Value,
) -> Result<u64, String> {
    use chord_core::permissions::{Permissions, RolePermissions, resolve_guild_base_permissions};
    let roles = array(data, "roles")?
        .iter()
        .map(|role| {
            let id = required_id(role, "id")?;
            let id = model::validate_snowflake(&id).map_err(|error| error.to_string())?;
            let decimal = role
                .get("permissions")
                .and_then(Value::as_str)
                .ok_or_else(|| "Guild role is missing its permissions.".to_string())?;
            let permissions = Permissions::from_decimal_str(decimal)
                .map_err(|_| "Guild role has invalid permission bits.".to_string())?
                .0;
            Ok(RolePermissions { id, permissions })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let member_roles = array(member, "roles")?
        .iter()
        .map(|id| {
            id.as_str()
                .ok_or_else(|| "Invalid member role ID.".to_string())
                .and_then(|id| model::validate_snowflake(id).map_err(|error| error.to_string()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(resolve_guild_base_permissions(
        guild,
        owner,
        user,
        &roles,
        &member_roles,
    ))
}

fn parse_audit_response(body: &[u8]) -> Result<Vec<AuditLogEntryRecord>, String> {
    let data: Value =
        serde_json::from_slice(body).map_err(|_| "Invalid audit log JSON.".to_string())?;
    let users = array(&data, "users")?
        .iter()
        .map(|user| {
            let id = required_id(user, "id")?;
            let name = user
                .get("global_name")
                .and_then(Value::as_str)
                .or_else(|| user.get("username").and_then(Value::as_str))
                .unwrap_or("Неизвестный")
                .to_string();
            Ok((id, name))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    array(&data, "audit_log_entries")?
        .iter()
        .map(|entry| parse_entry(entry, &users))
        .collect()
}

fn parse_entry(
    value: &Value,
    users: &BTreeMap<String, String>,
) -> Result<AuditLogEntryRecord, String> {
    let id = required_id(value, "id")?;
    let id = model::validate_snowflake(&id).map_err(|error| error.to_string())?;
    let action_type = value
        .get("action_type")
        .and_then(Value::as_u64)
        .and_then(|action| u16::try_from(action).ok())
        .ok_or_else(|| "Audit entry has invalid action_type.".to_string())?;
    let user_id = optional_id(value, "user_id")?;
    let target_id = optional_id(value, "target_id")?;
    let changes = parse_changes(value)?;
    Ok(AuditLogEntryRecord {
        id,
        actor_name: model::resolve_actor_name(user_id.as_deref(), users),
        user_id,
        target_name: target_id
            .as_ref()
            .map(|target| users.get(target).cloned().unwrap_or_else(|| target.clone())),
        target_id,
        action_type,
        reason: value
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        timestamp_ms: model::extract_snowflake_timestamp_ms(id),
        changes,
    })
}

fn parse_changes(value: &Value) -> Result<Vec<AuditChangeItem>, String> {
    match value.get("changes") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => values.iter().map(parse_change).collect(),
        _ => Err("Audit changes must be an array.".to_string()),
    }
}

fn parse_change(value: &Value) -> Result<AuditChangeItem, String> {
    let key = value
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| "Audit change is missing its key.".to_string())?;
    Ok(AuditChangeItem {
        key: key.to_string(),
        old_value: change_value(value.get("old_value")),
        new_value: change_value(value.get("new_value")),
    })
}

fn change_value(value: Option<&Value>) -> AuditChangeValue {
    match value {
        None => AuditChangeValue::Absent,
        Some(Value::Null) => AuditChangeValue::Null,
        Some(Value::Bool(value)) => AuditChangeValue::Bool(*value),
        Some(Value::Number(value)) if value.as_i64().is_some() => {
            AuditChangeValue::Number(value.as_i64().unwrap_or_default())
        }
        Some(Value::String(value)) => AuditChangeValue::Text(value.clone()),
        Some(value) => AuditChangeValue::Text(value.to_string()),
    }
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Missing or invalid {key} array."))
}

fn required_id(value: &Value, key: &str) -> Result<String, String> {
    optional_id(value, key)?.ok_or_else(|| format!("Missing {key}."))
}

fn optional_id(value: &Value, key: &str) -> Result<Option<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => {
            model::validate_snowflake(id).map_err(|error| error.to_string())?;
            Ok(Some(id.clone()))
        }
        _ => Err(format!("Invalid {key}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuditChangeValue, AuditLogPanel, AuditReply, guild_permissions, parse_audit_response,
    };

    #[test]
    fn parses_actual_response_fields_and_rejects_malformed_ids() {
        let body = br#"{"users":[{"id":"1","username":"moderator"}],"audit_log_entries":[
            {"id":"3","user_id":"1","target_id":null,"action_type":11,
            "changes":[{"key":"nsfw","old_value":false,"new_value":true}]}]}"#;
        let entries = parse_audit_response(body).expect("valid response");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor_name, "moderator");
        assert_eq!(
            entries[0].changes[0].old_value,
            AuditChangeValue::Bool(false)
        );
        for malformed in ["0", "18446744073709551616", "abc"] {
            let data = format!(
                r#"{{"users":[],"audit_log_entries":[{{"id":"{malformed}","action_type":1}}]}}"#
            );
            assert!(parse_audit_response(data.as_bytes()).is_err());
        }
        assert!(parse_audit_response(br#"{"users":[]}"#).is_err());
    }

    #[test]
    fn permission_gate_uses_current_member_roles() {
        let guild = serde_json::json!({"roles":[
            {"id":"1","permissions":"0"}, {"id":"2","permissions":"128"}
        ]});
        assert_eq!(
            guild_permissions(1, 9, 5, &guild, &serde_json::json!({"roles":["2"]}))
                .expect("audit role"),
            128
        );
        assert_eq!(
            guild_permissions(1, 9, 5, &guild, &serde_json::json!({"roles":[]}))
                .expect("no audit role"),
            0
        );
        assert!(guild_permissions(1, 9, 5, &guild, &serde_json::json!({"roles":["0"]})).is_err());
    }
    #[test]
    fn changing_guild_or_account_drops_stale_replies_and_clears_rows() {
        for (guild, account) in [(Some("2"), "10"), (Some("1"), "20"), (None, "10")] {
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut panel = AuditLogPanel {
                is_open: true,
                guild_id: Some("1".to_string()),
                account_id: Some("10".to_string()),
                receiver: Some(receiver),
                ..Default::default()
            };
            let entries = parse_audit_response(
                br#"{"users":[],"audit_log_entries":[{"id":"3","action_type":1}]}"#,
            )
            .expect("populated panel fixture");
            panel.viewer.replace_entries(entries).expect("valid entry");
            panel.viewer.toggle_row_expanded(3);
            assert_eq!(panel.viewer.rows().len(), 1);
            assert!(panel.retain_context(Some("1"), "10"));
            assert!(!panel.retain_context(guild, account));
            assert!(!panel.is_open && panel.viewer.rows().is_empty());
            assert!(panel.viewer.expanded_entry_ids.is_empty());
            assert!(
                sender
                    .send(Ok(AuditReply {
                        status: 200,
                        entries: Vec::new(),
                        retry_after_ms: None
                    }))
                    .is_err(),
                "a stale request must have no receiver"
            );
        }
    }
}
