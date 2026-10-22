use hidapi::HidApi;

const WACOM_VID: u16 = 0x056A;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn main() {
    let api = HidApi::new().expect("hidapi init");
    let devices: Vec<_> = api.device_list().filter(|d| d.vendor_id() == WACOM_VID).collect();

    let arg = std::env::args().nth(1);
    let Some(arg) = arg else {
        for (i, d) in devices.iter().enumerate() {
            println!(
                "[{i}] pid={:04x} usage_page={:04x} usage={:04x} path={}",
                d.product_id(),
                d.usage_page(),
                d.usage(),
                d.path().to_string_lossy()
            );
        }
        println!("usage: wacom <index>   dump descriptor and print raw reports");
        return;
    };

    let d = devices[arg.parse::<usize>().expect("index")];
    let dev = d.open_device(&api).expect("open failed (another process holds it exclusively?)");

    let mut desc = [0u8; 4096];
    match dev.get_report_descriptor(&mut desc) {
        Ok(n) => println!("descriptor ({n} bytes):\n{}", hex(&desc[..n])),
        Err(e) => println!("descriptor unavailable: {e}"),
    }

    println!("reading, move the pen (ctrl+c to stop)");
    let mut buf = [0u8; 256];
    loop {
        let n = dev.read(&mut buf).expect("read");
        println!("{}", hex(&buf[..n]));
    }
}
