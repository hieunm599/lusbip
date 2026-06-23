#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDeviceSummary {
    pub bus_id: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
}

pub async fn list_usb_devices() -> Result<Vec<UsbDeviceSummary>, String> {
    let devices = nusb::list_devices()
        .await
        .map_err(|err| format!("Failed to list USB devices: {err}"))?;

    Ok(devices
        .map(|device| UsbDeviceSummary {
            bus_id: device.bus_id().to_string(),
            vendor_id: device.vendor_id(),
            product_id: device.product_id(),
            manufacturer: device
                .manufacturer_string()
                .unwrap_or("Unknown")
                .to_string(),
            product: device.product_string().unwrap_or("Unknown").to_string(),
            serial: device.serial_number().unwrap_or("N/A").to_string(),
        })
        .collect())
}

pub async fn print_usb_devices() -> Result<(), String> {
    let devices = list_usb_devices().await?;

    println!(
        "{:<16} {:<10} {:<22} {:<30} Serial",
        "Bus ID", "VID:PID", "Manufacturer", "Product"
    );
    println!("{}", "-".repeat(96));
    for device in devices {
        println!(
            "{:<16} {:04x}:{:04x}   {:<22} {:<30} {}",
            device.bus_id,
            device.vendor_id,
            device.product_id,
            device.manufacturer,
            device.product,
            device.serial
        );
    }

    Ok(())
}

pub fn matches_filter(
    device: &UsbDeviceSummary,
    vid: Option<u16>,
    pid: Option<u16>,
    bus_id: Option<&str>,
) -> bool {
    if let Some(expected) = vid
        && device.vendor_id != expected
    {
        return false;
    }

    if let Some(expected) = pid
        && device.product_id != expected
    {
        return false;
    }

    if let Some(expected) = bus_id
        && device.bus_id != expected
    {
        return false;
    }

    true
}
