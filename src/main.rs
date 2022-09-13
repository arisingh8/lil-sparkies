#[allow(unused_imports)]
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{sync::Arc, thread, time::*};

use anyhow::{Context as _, Result, bail};

use log::*;

use futures::sink::{Sink, SinkExt};
use smol::{prelude::*, Async};
use tungstenite::Message;
use async_tungstenite::WebSocketStream;

use embedded_hal::digital::v2::OutputPin;

use embedded_svc::wifi::*;

use esp_idf_svc::netif::*;
use esp_idf_svc::nvs::*;
use esp_idf_svc::sysloop::*;
use esp_idf_svc::wifi::*;

use esp_idf_hal::prelude::*;

fn main() -> Result<()> {
    // Temporary. Will disappear once ESP-IDF 4.4 is released, but for now it is necessary to call this function once,
    // or else some patches to the runtime implemented by esp-idf-sys might not link properly.
    esp_idf_sys::link_patches();

    println!("Hello, world!");

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    #[allow(unused)]
    let peripherals = Peripherals::take().unwrap();
    #[allow(unused)]
    let pins = peripherals.pins;

    #[allow(unused)]
    let netif_stack = Arc::new(EspNetifStack::new()?);
    #[allow(unused)]
    let sys_loop_stack = Arc::new(EspSysLoopStack::new()?);
    #[allow(unused)]
    let default_nvs = Arc::new(EspDefaultNvs::new()?);

    // Turn on onboard LED
    let mut onboard_led = pins.gpio2.into_output()?;
    onboard_led.set_high()?;

    #[allow(clippy::redundant_clone)]
    let mut wifi = Box::new(EspWifi::new(netif_stack.clone(), sys_loop_stack.clone(), default_nvs.clone())?);

    info!("Wifi created, about to setup AP");

    let conf = Configuration::AccessPoint(AccessPointConfiguration {
        ssid: "esp_tester".into(),
        channel: 1,
        ..Default::default()
    });

    wifi.set_configuration(&conf)?;

    info!("Wifi configuration completed, about to wait for status");

    wifi.wait_status_with_timeout(Duration::from_secs(20), |status| !status.is_transitional())
        .map_err(|e| anyhow::anyhow!("Unexpected Wifi status: {:?}", e))?;

    let status = wifi.get_status();

    if let Status(ClientStatus::Stopped, ApStatus::Started(ApIpStatus::Done)) = status {
        info!("AP started");
    } else {
        bail!("Unexpected Wifi status: {:?}", status);
    }

    info!("About to bind a simple echo service to port 8081 using async (smol-rs)!");

    #[allow(clippy::needless_update)]
    {
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::esp_vfs_eventfd_register(&esp_idf_sys::esp_vfs_eventfd_config_t {
                max_fds: 5,
                ..Default::default()
            })
        })?;
    }

    let ws_handle = thread::Builder::new().stack_size(8192).spawn(|| {
        smol::block_on(ws_bind()).unwrap();
    })?;

    ws_handle.join().unwrap();

    Ok(())
}

async fn ws_bind() -> anyhow::Result<()> {
    /// Echoes messages from the client back to it.
    async fn echo(mut stream: WsStream) -> anyhow::Result<()> {
        let msg = stream.next().await.context("expected a message")??;
        stream.send(Message::text(msg.to_string())).await?;
        Ok(())
    }

    // Create a listener.
    let listener = smol::Async::<TcpListener>::bind(([0, 0, 0, 0], 8081))?;

    // Accept clients in a loop.
    loop {
        let (stream, peer_addr) = listener.accept().await?;
        info!("Accepted client: {}", peer_addr);

        // create the async WebSocketStream
        let stream = WsStream::Plain(async_tungstenite::accept_async(stream).await?);

        // Spawn a task that echoes messages from the client back to it.
        smol::spawn(echo(stream)).detach();
    }
}

/// A WebSocket or WebSocket+TLS connection.
enum WsStream {
    /// A plain WebSocket connection.
    Plain(WebSocketStream<Async<TcpStream>>),

    // A WebSocket connection secured by TLS.
    // Tls(WebSocketStream<TlsStream<Async<TcpStream>>>),
}

impl Sink<Message> for WsStream {
    type Error = tungstenite::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match &mut *self {
            WsStream::Plain(s) => Pin::new(s).poll_ready(cx),
            //WsStream::Tls(s) => Pin::new(s).poll_ready(cx),
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        match &mut *self {
            WsStream::Plain(s) => Pin::new(s).start_send(item),
            //WsStream::Tls(s) => Pin::new(s).start_send(item),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match &mut *self {
            WsStream::Plain(s) => Pin::new(s).poll_flush(cx),
            //WsStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match &mut *self {
            WsStream::Plain(s) => Pin::new(s).poll_close(cx),
            //WsStream::Tls(s) => Pin::new(s).poll_close(cx),
        }
    }
}

impl Stream for WsStream {
    type Item = tungstenite::Result<Message>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match &mut *self {
            WsStream::Plain(s) => Pin::new(s).poll_next(cx),
            //WsStream::Tls(s) => Pin::new(s).poll_next(cx),
        }
    }
}