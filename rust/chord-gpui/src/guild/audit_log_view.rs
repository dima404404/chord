#![forbid(unsafe_code)]
//! Filterable audit presentation, shared by the native panel and its contracts.
//!
//! Local provenance: TASK-1008/1008B and docs/re/frontend-d03.json describe
//! scrollerBase_d125d2, listItems_affa7e, menu_c1e9c4, comboBoxInput__0edde,
//! numberBadge__463b7, iconBadge__463b7 and avatar_fdd9da. Module references
//! auditLogSection_b2f52f, auditLog__43dab, GuildSettingsAuditLogStore and
//! AuditLogUtils identify that evidence, not copied styles or implementation.

use super::audit_log::{
    self as model, AuditActionCategory, AuditChangeItem, AuditLogEntryRecord, AuditLogError,
    AuditTargetType,
};
use std::collections::{BTreeMap, BTreeSet};

pub const AUDIT_LOG_VIEWER_CONTAINER: &str = "Audit Log Viewer";
pub const AUDIT_LOG_VIEWER_DIMENSIONS: &str = "100% x 100%";
pub const AUDIT_LOG_VIEWER_PADDING_PX: u32 = 16;
pub const AUDIT_LOG_VIEWER_SPACING_PX: u32 = 8;
pub const AUDIT_ROW_ELEMENT: &str = "audit-row";
pub const BACKGROUND: &str = "#313338";
pub const SURFACE: &str = "#2b2d31";
pub const TEXT: &str = "#ffffff";
pub const ACCENT: &str = "#5865f2";
pub const BORDER: &str = "#3f4147";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditRowInteractiveState {
    Summary,
    HoverRow,
    ExpandedDiff,
}

impl AuditRowInteractiveState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::HoverRow => "hover-row",
            Self::ExpandedDiff => "expanded diff",
        }
    }

    pub const fn resolve(expanded: bool, hovered: bool) -> Self {
        if expanded {
            Self::ExpandedDiff
        } else if hovered {
            Self::HoverRow
        } else {
            Self::Summary
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditActionIconSpec {
    pub glyph: &'static str,
    pub category: AuditActionCategory,
    pub target: AuditTargetType,
    pub badge_color: &'static str,
}

pub fn resolve_action_icon(action_type: u16) -> AuditActionIconSpec {
    let category = AuditActionCategory::from_action_code(action_type);
    let (glyph, badge_color) = match category {
        AuditActionCategory::Create => ("+", "#23a55a"),
        AuditActionCategory::Update => ("~", ACCENT),
        AuditActionCategory::Delete => ("−", "#f23f43"),
        AuditActionCategory::All => ("?", "#949ba4"),
    };
    AuditActionIconSpec {
        glyph,
        category,
        target: AuditTargetType::from_action_code(action_type),
        badge_color,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDropdownOption {
    pub action_type: Option<u16>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDropdownOption {
    pub user_id: Option<String>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLogRowViewSpec {
    pub entry_id: u64,
    pub icon: AuditActionIconSpec,
    pub moderator_handle: String,
    pub event_description_ru: String,
    pub timestamp_iso: String,
    pub timestamp_ms: u64,
    pub state: AuditRowInteractiveState,
    pub focused: bool,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AuditLoadState {
    #[default]
    Idle,
    Loading,
    Ready,
    Deferred,
    Forbidden,
    Failed(u16),
}

#[derive(Debug, Clone, Default)]
pub struct AuditLogViewer {
    entries: Vec<AuditLogEntryRecord>,
    pub selected_action_filter: Option<u16>,
    pub selected_user_filter: Option<String>,
    pub expanded_entry_ids: BTreeSet<u64>,
    pub focused_entry_id: Option<u64>,
    pub load_state: AuditLoadState,
    pub retry_after_ms: Option<u64>,
    pub error_message: Option<String>,
}

pub fn build_audit_log_viewer(
    entries: Vec<AuditLogEntryRecord>,
) -> Result<AuditLogViewer, AuditLogError> {
    let mut viewer = AuditLogViewer::default();
    viewer.replace_entries(entries)?;
    Ok(viewer)
}

impl AuditLogViewer {
    pub fn replace_entries(
        &mut self,
        mut entries: Vec<AuditLogEntryRecord>,
    ) -> Result<(), AuditLogError> {
        for entry in &mut entries {
            model::validate_snowflake(&entry.id.to_string())?;
            entry.timestamp_ms = model::extract_snowflake_timestamp_ms(entry.id);
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.id));
        self.entries = entries;
        let present: BTreeSet<_> = self.entries.iter().map(|entry| entry.id).collect();
        self.expanded_entry_ids.retain(|id| present.contains(id));
        self.load_state = AuditLoadState::Ready;
        self.error_message = None;
        self.retry_after_ms = None;
        self.focus_first();
        Ok(())
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn check_access(
        &mut self,
        permissions: u64,
        owner: bool,
        age_restricted: bool,
    ) -> Result<(), AuditLogError> {
        let result = model::check_audit_log_view_permission(permissions, owner, age_restricted);
        if result.is_err() {
            self.clear();
            self.load_state = AuditLoadState::Forbidden;
        }
        result
    }

    pub fn begin_loading(&mut self) {
        self.load_state = AuditLoadState::Loading;
        self.error_message = None;
    }

    pub fn apply_http_status(&mut self, status: u16, retry_after_ms: Option<u64>) {
        self.retry_after_ms = retry_after_ms;
        self.load_state = match status {
            200 => AuditLoadState::Ready,
            202 => AuditLoadState::Deferred,
            401 | 403 => {
                self.clear();
                AuditLoadState::Forbidden
            }
            other => AuditLoadState::Failed(other),
        };
    }

    pub fn set_error(&mut self, error: &str) {
        self.load_state = AuditLoadState::Failed(0);
        self.error_message = Some(model::redact_audit_diagnostic(error));
    }

    pub fn action_options(&self) -> Vec<ActionDropdownOption> {
        let mut options = vec![ActionDropdownOption {
            action_type: None,
            label: "Все действия".to_string(),
        }];
        let codes: BTreeSet<_> = self.entries.iter().map(|entry| entry.action_type).collect();
        options.extend(codes.into_iter().map(|code| ActionDropdownOption {
            action_type: Some(code),
            label: model::audit_action_label_ru(code),
        }));
        options
    }

    pub fn user_options(&self) -> Vec<UserDropdownOption> {
        let mut users = BTreeMap::new();
        for entry in &self.entries {
            if let Some(id) = &entry.user_id {
                users.entry(id.clone()).or_insert_with(|| actor_name(entry));
            }
        }
        let mut options = vec![UserDropdownOption {
            user_id: None,
            label: "Все пользователи".to_string(),
        }];
        options.extend(
            users
                .into_iter()
                .map(|(user_id, label)| UserDropdownOption {
                    user_id: Some(user_id),
                    label,
                }),
        );
        options
    }

    pub fn filter_by_action(&mut self, action: Option<u16>) {
        self.selected_action_filter = action;
        self.focus_first();
    }

    pub fn filter_by_user(&mut self, user: Option<String>) -> Result<(), AuditLogError> {
        if let Some(id) = &user {
            model::validate_snowflake(id)?;
        }
        self.selected_user_filter = user;
        self.focus_first();
        Ok(())
    }

    pub fn filtered_entries(&self) -> Vec<&AuditLogEntryRecord> {
        self.entries
            .iter()
            .filter(|entry| {
                if let Some(act) = self.selected_action_filter {
                    if entry.action_type != act {
                        return false;
                    }
                }
                if let Some(uid) = &self.selected_user_filter {
                    if entry.user_id.as_ref() != Some(uid) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    pub fn rows(&self) -> Vec<AuditLogRowViewSpec> {
        self.filtered_entries()
            .into_iter()
            .map(|entry| AuditLogRowViewSpec {
                entry_id: entry.id,
                icon: resolve_action_icon(entry.action_type),
                moderator_handle: actor_name(entry),
                event_description_ru: model::redact_audit_diagnostic(&entry.summary_ru()),
                timestamp_iso: format_timestamp_iso(entry.timestamp_ms),
                timestamp_ms: entry.timestamp_ms,
                state: AuditRowInteractiveState::resolve(
                    self.expanded_entry_ids.contains(&entry.id),
                    false,
                ),
                focused: self.focused_entry_id == Some(entry.id),
                details: entry.changes.iter().map(format_change_diff).collect(),
            })
            .collect()
    }

    pub fn toggle_row_expanded(&mut self, entry_id: u64) -> bool {
        if !self.entries.iter().any(|entry| entry.id == entry_id) {
            return false;
        }
        if !self.expanded_entry_ids.remove(&entry_id) {
            self.expanded_entry_ids.insert(entry_id);
        }
        self.expanded_entry_ids.contains(&entry_id)
    }

    pub fn handle_key(&mut self, key: &str) {
        let ids: Vec<_> = self
            .filtered_entries()
            .iter()
            .map(|entry| entry.id)
            .collect();
        if ids.is_empty() {
            self.focused_entry_id = None;
            return;
        }
        let current = self
            .focused_entry_id
            .and_then(|id| ids.iter().position(|i| *i == id));
        let index = match key {
            "ArrowDown" | "down" => current.map_or(0, |i| (i + 1).min(ids.len() - 1)),
            "ArrowUp" | "up" => current.unwrap_or(0).saturating_sub(1),
            "Home" | "home" => 0,
            "End" | "end" => ids.len() - 1,
            "Enter" | "enter" | "Space" | "space" | " " => {
                if let Some(id) = self.focused_entry_id {
                    self.toggle_row_expanded(id);
                }
                return;
            }
            _ => return,
        };
        self.focused_entry_id = ids.get(index).copied();
    }

    pub fn grouped_entries(&self) -> Vec<Vec<u64>> {
        let mut groups: Vec<Vec<u64>> = Vec::new();
        let mut previous: Option<&AuditLogEntryRecord> = None;
        for entry in self.filtered_entries() {
            if let (Some(prev), Some(group)) = (previous, groups.last_mut()) {
                if model::can_merge_audit_entries(prev, entry, group.len()) {
                    group.push(entry.id);
                    previous = Some(entry);
                    continue;
                }
            }
            groups.push(vec![entry.id]);
            previous = Some(entry);
        }
        groups
    }

    fn focus_first(&mut self) {
        self.focused_entry_id = self.filtered_entries().first().map(|entry| entry.id);
    }
}

fn actor_name(entry: &AuditLogEntryRecord) -> String {
    let name = entry.actor_name.trim();
    model::redact_audit_diagnostic(if name.is_empty() {
        "Неизвестный"
    } else {
        name
    })
}

pub fn format_change_diff(change: &AuditChangeItem) -> String {
    let key = &change.key;
    let old_str = model::render_audit_change_value(key, &change.old_value);
    let new_str = model::render_audit_change_value(key, &change.new_value);
    format!("{key}: {old_str} → {new_str}")
}

/// UTC Gregorian date from Unix days; Snowflake timestamps fit the signed intermediates.
pub fn format_timestamp_iso(timestamp_ms: u64) -> String {
    let seconds = timestamp_ms / 1_000;
    let days = seconds / 86_400;
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = seconds / 3_600 % 24;
    let minute = seconds / 60 % 60;
    let second = seconds % 60;
    let millis = timestamp_ms % 1_000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{AuditChangeValue as Value, PERM_ADMINISTRATOR, PERM_VIEW_AUDIT_LOG};

    fn entry(id: u64, user: &str, action: u16) -> AuditLogEntryRecord {
        AuditLogEntryRecord {
            id,
            user_id: Some(user.to_string()),
            actor_name: "Админ".to_string(),
            target_id: Some("5".to_string()),
            target_name: Some("Боб".to_string()),
            action_type: action,
            reason: Some("Спам".to_string()),
            timestamp_ms: 0,
            changes: vec![AuditChangeItem {
                key: "name".to_string(),
                old_value: Value::Text("старый-канал".to_string()),
                new_value: Value::Text("новый-канал".to_string()),
            }],
        }
    }

    fn viewer() -> AuditLogViewer {
        build_audit_log_viewer(vec![
            entry(1, "100", 10),
            entry(2, "200", 22),
            entry(3, "100", 11),
        ])
        .expect("valid audit entries")
    }

    #[test]
    fn test_sub_1008b_1_layout_and_interactive_states() {
        assert_eq!(AUDIT_LOG_VIEWER_CONTAINER, "Audit Log Viewer");
        assert_eq!(AUDIT_LOG_VIEWER_DIMENSIONS, "100% x 100%");
        assert_eq!(
            (AUDIT_LOG_VIEWER_PADDING_PX, AUDIT_LOG_VIEWER_SPACING_PX),
            (16, 8)
        );
        assert_eq!(
            [BACKGROUND, SURFACE, TEXT, ACCENT, BORDER],
            ["#313338", "#2b2d31", "#ffffff", "#5865f2", "#3f4147"]
        );
        assert_eq!(
            AuditRowInteractiveState::resolve(false, false).as_str(),
            "summary"
        );
        assert_eq!(
            AuditRowInteractiveState::resolve(false, true).as_str(),
            "hover-row"
        );
        assert_eq!(
            AuditRowInteractiveState::resolve(true, true).as_str(),
            "expanded diff"
        );
        assert_eq!(
            model::AuditEntryInteractiveState::ExpandedDetails.as_str(),
            "expanded details"
        );
    }

    #[test]
    fn test_sub_1008b_2_rows_icons_labels_and_snowflake_time() {
        for (code, label) in [
            (1, "Обновление сервера"),
            (10, "Создание канала"),
            (11, "Изменение канала"),
            (12, "Удаление канала"),
            (20, "Исключение участника"),
            (22, "Бан участника"),
            (23, "Разбан участника"),
            (30, "Создание роли"),
            (31, "Изменение роли"),
            (32, "Удаление роли"),
            (72, "Удаление сообщения"),
        ] {
            assert_eq!(model::audit_action_label_ru(code), label);
        }
        let rows = viewer().rows();
        assert_eq!(rows[1].moderator_handle, "Админ");
        assert_eq!(
            rows[1].event_description_ru,
            "Админ: Бан участника (Боб) — Причина: Спам"
        );
        assert_eq!(rows[1].timestamp_ms, model::DISCORD_EPOCH_MS);
        assert_eq!(rows[1].timestamp_iso, "2015-01-01T00:00:00.000Z");
        assert_eq!(resolve_action_icon(10).badge_color, "#23a55a");
        assert_eq!(resolve_action_icon(11).badge_color, "#5865f2");
        assert_eq!(resolve_action_icon(12).badge_color, "#f23f43");
        assert_eq!(resolve_action_icon(10).target, AuditTargetType::Channel);
        assert_eq!(resolve_action_icon(999).category, AuditActionCategory::All);
        assert_eq!(
            format_timestamp_iso(model::extract_snowflake_timestamp_ms(u64::MAX)),
            "2154-05-15T07:35:11.103Z"
        );
    }

    #[test]
    fn test_sub_1008b_3_action_and_user_dropdowns_filter_real_rows() {
        let mut view = viewer();
        assert_eq!(view.action_options().len(), 4);
        assert_eq!(view.user_options().len(), 3);
        view.filter_by_action(Some(10));
        assert_eq!(
            view.rows().iter().map(|r| r.entry_id).collect::<Vec<_>>(),
            vec![1]
        );
        view.filter_by_user(Some("200".to_string()))
            .expect("valid user");
        assert!(view.rows().is_empty());
        view.filter_by_action(None);
        assert_eq!(
            view.rows().iter().map(|r| r.entry_id).collect::<Vec<_>>(),
            vec![2]
        );
        view.filter_by_user(None).expect("all users");
        assert_eq!(view.rows().len(), 3);
        view.filter_by_user(Some("0".to_string()))
            .expect_err("zero user is invalid");
    }

    #[test]
    fn test_sub_1008b_3_click_expansion_and_keyboard_navigation() {
        let mut view = viewer();
        assert!(view.toggle_row_expanded(3));
        assert_eq!(view.rows()[0].state.as_str(), "expanded diff");
        assert_eq!(view.rows()[0].details, ["name: старый-канал → новый-канал"]);
        assert!(!view.toggle_row_expanded(3));
        assert_eq!(view.rows()[0].state.as_str(), "summary");
        assert!(!view.toggle_row_expanded(999));
        view.handle_key("End");
        assert_eq!(view.focused_entry_id, Some(1));
        view.handle_key("ArrowUp");
        assert_eq!(view.focused_entry_id, Some(2));
        view.handle_key("Home");
        assert_eq!(view.focused_entry_id, Some(3));
        view.handle_key("ArrowDown");
        view.handle_key("Enter");
        assert!(view.expanded_entry_ids.contains(&2));
        view.handle_key("Space");
        assert!(!view.expanded_entry_ids.contains(&2));
        view.filter_by_action(Some(999));
        view.handle_key("ArrowDown");
        assert_eq!(view.focused_entry_id, None);
    }

    #[test]
    fn test_sub_1008b_4_access_errors_empty_loading_and_recovery() {
        let mut view = viewer();
        view.check_access(0, false, false)
            .expect_err("no VIEW_AUDIT_LOG");
        assert!(view.rows().is_empty());
        assert_eq!(view.load_state, AuditLoadState::Forbidden);
        view.check_access(PERM_VIEW_AUDIT_LOG, false, false)
            .expect("audit permission");
        view.check_access(PERM_ADMINISTRATOR, false, false)
            .expect("administrator");
        view.check_access(0, true, false).expect("owner");
        view.check_access(PERM_ADMINISTRATOR, true, true)
            .expect_err("AGE_GATED_SPACES");
        view.begin_loading();
        assert_eq!(view.load_state, AuditLoadState::Loading);
        view.apply_http_status(202, None);
        assert_eq!(view.load_state, AuditLoadState::Deferred);
        for status in [400, 404, 413, 429, 500, 503] {
            view.apply_http_status(status, Some(1_234));
            assert_eq!(view.load_state, AuditLoadState::Failed(status));
            assert_eq!(view.retry_after_ms, Some(1_234));
        }
        view.replace_entries(vec![entry(1, "100", 10)])
            .expect("recovery");
        view.toggle_row_expanded(1);
        view.apply_http_status(403, None);
        assert!(view.rows().is_empty() && view.expanded_entry_ids.is_empty());
        assert_eq!(view.load_state, AuditLoadState::Forbidden);
        view.apply_http_status(401, None);
        assert_eq!(view.load_state, AuditLoadState::Forbidden);
        view.replace_entries(vec![]).expect("empty audit response");
        assert!(view.rows().is_empty());
        assert_eq!(view.load_state, AuditLoadState::Ready);
    }

    #[test]
    fn test_sub_1008b_4_bounds_grouping_and_redaction() {
        assert!(build_audit_log_viewer(vec![entry(0, "100", 1)]).is_err());
        assert!(build_audit_log_viewer(vec![entry(u64::MAX, "100", 1)]).is_ok());
        for limit in [0, 101] {
            let query = model::GuildAuditLogQuery {
                guild_id: "1".to_string(),
                limit: Some(limit),
                ..Default::default()
            };
            assert!(model::build_guild_audit_log_request(&query).is_err());
        }
        for limit in [1, 100] {
            let query = model::GuildAuditLogQuery {
                guild_id: "1".to_string(),
                limit: Some(limit),
                ..Default::default()
            };
            assert!(model::build_guild_audit_log_request(&query).is_ok());
        }
        assert!(model::validate_audit_log_reason_header("").is_err());
        assert!(model::validate_audit_log_reason_header(&"я".repeat(512)).is_ok());
        assert!(model::validate_audit_log_reason_header(&"я".repeat(513)).is_err());
        assert!(model::validate_system_boundaries(2000, 10, 25 * 1024 * 1024, 9975, 200).is_ok());
        assert!(model::validate_system_boundaries(2001, 10, 0, 0, 0).is_err());
        assert!(model::validate_system_boundaries(0, 11, 0, 0, 0).is_err());
        assert!(model::validate_system_boundaries(0, 0, 25 * 1024 * 1024 + 1, 0, 0).is_err());
        assert!(model::validate_system_boundaries(0, 0, 0, 9976, 0).is_err());
        assert!(model::validate_system_boundaries(0, 0, 0, 0, 201).is_err());
        assert_eq!(model::format_badge_cap(100), "99+");
    }

    #[test]
    fn test_sub_1008b_4_grouping_and_redaction() {
        let mut first = entry(1, "100", 11);
        first.changes[0].key = "token".to_string();
        let secret = format_change_diff(&first.changes[0]);
        assert!(!secret.contains("старый-канал") && !secret.contains("новый-канал"));
        let next = entry(2, "100", 11);
        assert_eq!(
            build_audit_log_viewer(vec![first, next])
                .expect("group")
                .grouped_entries(),
            vec![vec![2, 1]]
        );
        let later_id = ((model::MERGE_WINDOW_MS + 1) << 22) + 1;
        let distant = entry(later_id, "100", 11);
        assert_eq!(
            build_audit_log_viewer(vec![entry(1, "100", 11), distant])
                .expect("two groups")
                .grouped_entries()
                .len(),
            2
        );
        let mut view = viewer();
        view.set_error("credential mfa.synthetic-test-value");
        assert!(
            !view
                .error_message
                .as_deref()
                .unwrap_or_default()
                .contains("synthetic-test-value")
        );
    }
}
