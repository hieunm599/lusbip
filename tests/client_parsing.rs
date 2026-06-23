use lusbip::client::{
    AttachTarget, AttachedUsbPort, RemoteUsbDevice, format_remote_device_state,
    parse_usbip_list_output, parse_usbip_port_output, parse_vhci_status_ports, ports_to_detach,
    ports_to_detach_for_missing_remote_devices, remote_device_states,
};

#[test]
fn parses_usbip_list_output_and_skips_headers() {
    let output = r#"
Exportable USB devices
======================
 - 10.10.61.72
      1-1: Silicon Labs : CP2102 USB to UART Bridge Controller (10c4:ea60)
           : /sys/devices/platform/soc/1-1
           : Vendor Specific Class / unknown subclass / unknown protocol (ff/00/00)

      1-2: Example Corp : Debug Probe (1234:5678)
"#;

    assert_eq!(
        parse_usbip_list_output(output),
        vec![
            RemoteUsbDevice {
                bus_id: "1-1".into(),
                description: "Silicon Labs : CP2102 USB to UART Bridge Controller (10c4:ea60)"
                    .into(),
            },
            RemoteUsbDevice {
                bus_id: "1-2".into(),
                description: "Example Corp : Debug Probe (1234:5678)".into(),
            },
        ]
    );
}

#[test]
fn parses_usbip_port_output_for_attached_ports() {
    let output = r#"
Imported USB devices
====================
Port 00: <Port in Use> at High Speed(480Mbps)
       Silicon Labs : CP2102 USB to UART Bridge Controller (10c4:ea60)
       3-1 -> usbip://10.10.61.72:3240/1-1
           -> remote bus/dev 001/002

Port 01: <Port in Use> at Full Speed(12Mbps)
       Other Device (1234:5678)
       3-2 -> usbip://10.10.1.5:3240/2-1
"#;

    assert_eq!(
        parse_usbip_port_output(output),
        vec![
            AttachedUsbPort {
                port: "00".into(),
                remote_host: Some("10.10.61.72".into()),
                remote_bus_id: Some("1-1".into()),
                vid_pid: Some("10c4:ea60".into()),
            },
            AttachedUsbPort {
                port: "01".into(),
                remote_host: Some("10.10.1.5".into()),
                remote_bus_id: Some("2-1".into()),
                vid_pid: Some("1234:5678".into()),
            },
        ]
    );
}

#[test]
fn selects_only_matching_ports_for_auto_detach() {
    let ports = vec![
        AttachedUsbPort {
            port: "00".into(),
            remote_host: Some("10.10.61.72".into()),
            remote_bus_id: Some("1-1".into()),
            vid_pid: Some("10c4:ea60".into()),
        },
        AttachedUsbPort {
            port: "01".into(),
            remote_host: Some("10.10.1.5".into()),
            remote_bus_id: Some("2-1".into()),
            vid_pid: Some("1234:5678".into()),
        },
    ];

    let target = AttachTarget {
        remote_host: "10.10.61.72".into(),
        bus_id: Some("1-1".into()),
        vid_pid: Some("10c4:ea60".into()),
    };

    assert_eq!(ports_to_detach(&ports, &target), vec!["00"]);
}

#[test]
fn detaches_unknown_host_port_when_vid_pid_matches_target() {
    let ports = vec![AttachedUsbPort {
        port: "00".into(),
        remote_host: None,
        remote_bus_id: None,
        vid_pid: Some("10c4:ea60".into()),
    }];

    let target = AttachTarget {
        remote_host: "10.10.61.72".into(),
        bus_id: Some("1-1".into()),
        vid_pid: Some("10c4:ea60".into()),
    };

    assert_eq!(ports_to_detach(&ports, &target), vec!["00"]);
}

#[test]
fn detaches_remote_ports_when_server_no_longer_exports_their_bus_id() {
    let devices = vec![RemoteUsbDevice {
        bus_id: "5-2".into(),
        description: "Example : Debug Probe (1234:5678)".into(),
    }];
    let ports = vec![
        AttachedUsbPort {
            port: "00".into(),
            remote_host: Some("10.10.61.72".into()),
            remote_bus_id: Some("5-1".into()),
            vid_pid: Some("10c4:ea60".into()),
        },
        AttachedUsbPort {
            port: "01".into(),
            remote_host: Some("10.10.1.5".into()),
            remote_bus_id: Some("5-1".into()),
            vid_pid: Some("10c4:ea60".into()),
        },
        AttachedUsbPort {
            port: "02".into(),
            remote_host: Some("10.10.61.72".into()),
            remote_bus_id: Some("5-2".into()),
            vid_pid: Some("1234:5678".into()),
        },
    ];

    assert_eq!(
        ports_to_detach_for_missing_remote_devices("10.10.61.72", &devices, &ports),
        vec!["00"]
    );
}

#[test]
fn detaches_all_ports_from_remote_when_server_exports_no_devices() {
    let ports = vec![
        AttachedUsbPort {
            port: "00".into(),
            remote_host: Some("10.10.61.72".into()),
            remote_bus_id: Some("5-1".into()),
            vid_pid: Some("10c4:ea60".into()),
        },
        AttachedUsbPort {
            port: "01".into(),
            remote_host: Some("10.10.1.5".into()),
            remote_bus_id: Some("2-1".into()),
            vid_pid: Some("1234:5678".into()),
        },
    ];

    assert_eq!(
        ports_to_detach_for_missing_remote_devices("10.10.61.72", &[], &ports),
        vec!["00"]
    );
}

#[test]
fn remote_device_states_mark_attached_devices_by_remote_bus_id() {
    let devices = vec![
        RemoteUsbDevice {
            bus_id: "5-1".into(),
            description: "Silicon Labs : CP210x UART Bridge (10c4:ea60)".into(),
        },
        RemoteUsbDevice {
            bus_id: "5-2".into(),
            description: "Example : Debug Probe (1234:5678)".into(),
        },
    ];
    let ports = vec![AttachedUsbPort {
        port: "00".into(),
        remote_host: Some("10.10.61.72".into()),
        remote_bus_id: Some("5-1".into()),
        vid_pid: Some("10c4:ea60".into()),
    }];

    let states = remote_device_states("10.10.61.72", &devices, &ports);

    assert_eq!(states[0].attached_port.as_deref(), Some("00"));
    assert_eq!(states[1].attached_port, None);
    assert_eq!(
        format_remote_device_state(&states[0]),
        "[x] port 00 | 5-1 | Silicon Labs : CP210x UART Bridge (10c4:ea60)"
    );
    assert_eq!(
        format_remote_device_state(&states[1]),
        "[ ] | 5-2 | Example : Debug Probe (1234:5678)"
    );
}

#[test]
fn remote_device_states_keep_attached_ports_when_remote_export_is_empty() {
    let ports = vec![AttachedUsbPort {
        port: "00".into(),
        remote_host: None,
        remote_bus_id: None,
        vid_pid: Some("10c4:ea60".into()),
    }];

    let states = remote_device_states("10.10.61.72", &[], &ports);

    assert_eq!(states.len(), 1);
    assert_eq!(states[0].attached_port.as_deref(), Some("00"));
    assert_eq!(states[0].device.bus_id, "attached-port-00");
    assert_eq!(
        format_remote_device_state(&states[0]),
        "[x] port 00 | attached-port-00 | Attached USB/IP device (10c4:ea60)"
    );
}

#[test]
fn parses_stale_vhci_ports_from_kernel_status() {
    let status = r#"
hub port sta spd dev      sockfd local_busid
hs  0000 006 002 0005000f 000003 5-1
hs  0001 004 000 00000000 000000 0-0
ss  0008 006 005 00080002 000003 6-1
ss  0009 004 000 00000000 000000 0-0
"#;

    assert_eq!(parse_vhci_status_ports(status), vec!["00", "08"]);
}
