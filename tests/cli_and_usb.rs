use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use lusbip::cli::parse_hex_u16;
use lusbip::client::{AttachedUsbPort, format_attached_port};
use lusbip::server::{SharedDeviceView, format_shared_device_row};
use lusbip::tui::{
    ListKeyAction, SelectionAction, TuiItem, label_with_spinner, list_key_action,
    merge_retained_items, next_index, optimistic_toggle_label, preserve_selected_index,
    should_flush_startup_event, spinner_frame, truncate_to_width,
};
use lusbip::usb::{UsbDeviceSummary, matches_filter};

#[test]
fn parses_hex_with_or_without_prefix() {
    assert_eq!(parse_hex_u16("10c4").unwrap(), 0x10c4);
    assert_eq!(parse_hex_u16("0xea60").unwrap(), 0xea60);
    assert!(parse_hex_u16("not-hex").is_err());
}

#[test]
fn matches_usb_device_filters_by_vid_pid_and_bus_id() {
    let device = UsbDeviceSummary {
        bus_id: "1-1".into(),
        vendor_id: 0x10c4,
        product_id: 0xea60,
        manufacturer: "Silicon Labs".into(),
        product: "CP2102".into(),
        serial: "n/a".into(),
    };

    assert!(matches_filter(
        &device,
        Some(0x10c4),
        Some(0xea60),
        Some("1-1")
    ));
    assert!(!matches_filter(&device, Some(0x1366), None, None));
    assert!(!matches_filter(&device, None, Some(0x0001), None));
    assert!(!matches_filter(&device, None, None, Some("2-1")));
}

#[test]
fn tui_index_wraps_for_navigation() {
    assert_eq!(next_index(0, 3, SelectionAction::Up), 2);
    assert_eq!(next_index(2, 3, SelectionAction::Down), 0);
    assert_eq!(next_index(1, 3, SelectionAction::Toggle), 1);
    assert_eq!(next_index(0, 0, SelectionAction::Down), 0);
}

#[test]
fn tui_preserves_selected_item_by_id_after_refresh_reorders_rows() {
    let previous = vec![
        TuiItem {
            id: "5-1".into(),
            label: "[ ] | 5-1 | CP2102".into(),
        },
        TuiItem {
            id: "8-1".into(),
            label: "[ ] | 8-1 | SanDisk".into(),
        },
    ];
    let next = vec![
        TuiItem {
            id: "8-1".into(),
            label: "[x] port 00 | 8-1 | SanDisk".into(),
        },
        TuiItem {
            id: "5-1".into(),
            label: "[ ] | 5-1 | CP2102".into(),
        },
    ];

    assert_eq!(preserve_selected_index(&previous, &next, 1), 0);
}

#[test]
fn tui_keeps_pending_items_when_refresh_temporarily_omits_them() {
    let previous = vec![
        TuiItem {
            id: "5-1".into(),
            label: "[ ] | 5-1 | CP2102".into(),
        },
        TuiItem {
            id: "8-1".into(),
            label: "[ ] | 8-1 | SanDisk".into(),
        },
    ];
    let next = vec![TuiItem {
        id: "5-1".into(),
        label: "[ ] | 5-1 | CP2102".into(),
    }];
    let retained = vec!["8-1".to_string()];

    let merged = merge_retained_items(&previous, &next, &retained);

    assert_eq!(merged.len(), 2);
    assert!(merged.iter().any(|item| item.id == "8-1"));
}

#[test]
fn tui_flushes_startup_enter_key_before_selection_loop() {
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(should_flush_startup_event(&enter));
}

#[test]
fn tui_maps_space_to_activate_and_enter_to_noop() {
    let space = Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(list_key_action(&space), Some(ListKeyAction::Activate));
    assert_eq!(list_key_action(&enter), None);
    assert_eq!(list_key_action(&esc), Some(ListKeyAction::Background));
}

#[test]
fn tui_spinner_frames_rotate_and_append_to_row_label() {
    assert_eq!(spinner_frame(0), "|");
    assert_eq!(spinner_frame(1), "/");
    assert_eq!(spinner_frame(2), "-");
    assert_eq!(spinner_frame(3), "\\");
    assert_eq!(spinner_frame(4), "|");
    assert_eq!(
        label_with_spinner("[ ] | 5-1 | CP2102", 1),
        "[ ] | 5-1 | CP2102  /"
    );
}

#[test]
fn tui_optimistically_updates_label_after_toggle_success() {
    assert_eq!(
        optimistic_toggle_label("[ ] | 5-1 | CP2102", true),
        "[x] | 5-1 | CP2102"
    );
    assert_eq!(
        optimistic_toggle_label("[x] port 00 | 5-1 | CP2102", false),
        "[ ] | 5-1 | CP2102"
    );
}

#[test]
fn tui_truncates_long_rows_to_fixed_width() {
    assert_eq!(truncate_to_width("CP2102 USB UART", 9), "CP2102...");
    assert_eq!(truncate_to_width("CP2102", 10), "CP2102");
    assert_eq!(truncate_to_width("CP2102", 3), "...");
    assert_eq!(truncate_to_width("CP2102", 0), "");
}

#[test]
fn attached_port_label_contains_detach_selection_details() {
    let port = AttachedUsbPort {
        port: "00".into(),
        remote_host: Some("10.10.61.72".into()),
        remote_bus_id: Some("5-1".into()),
        vid_pid: Some("10c4:ea60".into()),
    };

    assert_eq!(
        format_attached_port(&port),
        "Port 00 | host: 10.10.61.72 | bus: 5-1 | vid:pid: 10c4:ea60"
    );
}

#[test]
fn shared_device_row_shows_occupying_client_ip() {
    let device = SharedDeviceView {
        bus_id: "5-1".into(),
        vid_pid: "10c4:ea60".into(),
        product: "CP2102".into(),
        client: Some("10.10.60.208".into()),
    };

    assert_eq!(
        format_shared_device_row(&device),
        "5-1 | 10c4:ea60 | CP2102 | occupied by 10.10.60.208"
    );
}
