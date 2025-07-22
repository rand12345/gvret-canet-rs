use crate::{
    gvret::{Gvret, Mode, convert_to_gvret, decode_gvret_frames},
    usr_canet::{CanetMsg, Message, convert_to_canet, decode_canet_frame},
};
use clap::{Arg, Command, ValueEnum};
use env_logger::Env;
use log::*;
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    time::Instant,
};
mod gvret;
mod usr_canet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let matches = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(
            Arg::new("interface")
                .short('i')
                .long("interface")
                .value_name("INTERFACE")
                .help("Sets the bind interface (local or any)")
                .value_parser(clap::value_parser!(Interface))
                .default_value("local"),
        )
        .arg(
            Arg::new("ip")
                .index(1)
                .value_name("IP")
                .help("Sets the CANET IP address")
                .required(true),
        )
        .arg(
            Arg::new("port1")
                .index(2)
                .value_name("PORT1")
                .help("Sets CAN1 CANET TCP port")
                .value_parser(clap::value_parser!(u16))
                .required(true),
        )
        .arg(
            Arg::new("port2")
                .long("port2")
                .value_name("PORT2")
                .help("Sets CAN2 CANET port (optional)")
                .value_parser(clap::value_parser!(u16)),
        )
        .arg(
            Arg::new("debug")
                .short('d')
                .long("debug")
                .value_name("LEVEL")
                .help("Sets the debug level")
                .value_parser(clap::value_parser!(LevelFilter))
                .default_value("info"),
        )
        .get_matches();

    // Initialize logging
    let log_level = matches
        .get_one::<LevelFilter>("debug")
        .copied()
        .unwrap_or(LevelFilter::Info);

    env_logger::Builder::from_env(Env::default().default_filter_or(log_level.to_string())).init();

    let bind_addr = match matches.get_one::<Interface>("interface").unwrap() {
        Interface::Local => "127.0.0.1:23",
        Interface::Any => "0.0.0.0:23",
    };

    let ip = matches
        .get_one::<String>("ip")
        .expect("IP address is required")
        .to_string();
    let port1 = *matches
        .get_one::<u16>("port1")
        .expect("port1 must be provided");
    let port2 = matches.get_one::<u16>("port2").copied();

    info!("Starting local canet-rs server...");

    let gvret_listener = TcpListener::bind(bind_addr).await?;
    info!("Listening on {:?}", gvret_listener.local_addr().unwrap());

    let (gvret_stream, addr) = gvret_listener.accept().await?;
    info!("Accepted gvret client from {addr}");

    // Connect to CANET device
    let canet_stream1 = TcpStream::connect(format!("{ip}:{port1}")).await?;
    info!("Connected to CANET CAN1");

    // Optional second port
    let canet_stream2 = if let Some(port) = port2 {
        match TcpStream::connect(format!("{ip}:{port}")).await {
            Ok(s) => {
                info!("Connected to CANET CAN1");
                Some(s)
            }
            Err(e) => {
                error!("Connection to Canet CAN 2 failed {e}");
                None
            }
        }
    } else {
        None
    };

    // Split streams
    let (mut gvret_r, mut gvret_w) = gvret_stream.into_split();
    let (mut canet1_r, mut canet1_w) = canet_stream1.into_split();
    let (mut canet2_r, mut canet2_w, busses) = match canet_stream2 {
        Some(s) => {
            let (r, w) = s.into_split();
            (Some(r), Some(w), 2)
        }
        None => (None, None, 1),
    };

    let now = Instant::now();
    let mut mode = Mode::Init;
    loop {
        tokio::select! {
            // Handle gvret to canet
            result = decode_gvret_frames(&mut gvret_r, &mut mode, busses, now) => {
                match result {
                    Gvret::Frame(message) => {
                        let data = convert_to_canet(message);
                        match data {
                            CanetMsg::Can1(data) => {
                                canet1_w.write_all(&data).await?;
                                canet1_w.flush().await?;
                            }
                            CanetMsg::Can2(data) => {
                                if let Some(w) = canet2_w.as_mut(){
                                    w.write_all(&data).await?;
                                    w.flush().await?;
                                };

                            }
                        }
                    }
                    Gvret::Init(b) => {
                        gvret_w.write_all(&b).await?;
                        gvret_w.flush().await?;
                    }
                }
            }
            // Handle canet1 to gvret
            result = decode_canet_frame(&mut canet1_r, 0) => {
                if let Some(frame) = result {
                    if let Some(b) = convert_to_gvret(frame, now) {
                        gvret_w.write_all(&b).await?;
                        gvret_w.flush().await?;
                    }
                }
            }
            // Handle canet2 to gvret (if connected)
            result = async {
                if let Some(r) = canet2_r.as_mut(){
                    decode_canet_frame(r, 1).await
                } else {
                    std::future::pending::<Option<Message>>().await
                }
            } => {
                if let Some(frame) = result {
                    if let Some(b) = convert_to_gvret(frame, now) {
                        gvret_w.write_all(&b).await?;
                        gvret_w.flush().await?;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Interface {
    Local,
    Any,
}
