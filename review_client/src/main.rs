use bytes::Bytes;
use h2::client::SendRequest;
use http::{Request, Version};
use std::net::SocketAddrV4;
use std::str::FromStr;
use std::time::{Duration, Instant};
use structopt::StructOpt;
use tokio::net::TcpStream;

fn parse_requests(src: &str) -> anyhow::Result<u8> {
    use anyhow::anyhow;

    let value = u8::from_str(src)?;
    if (1..=100).contains(&value) {
        Ok(value)
    } else {
        Err(anyhow!("number has to be between 1 and 100"))
    }
}

#[derive(Debug, StructOpt)]
struct Args {
    #[structopt(short, long, default_value = "127.0.0.1:8080", help = "Адрес сервера")]
    addr: SocketAddrV4,
    #[structopt(short, long, parse(try_from_str = parse_requests), default_value = "10", help = "Количество запросов = [1..100]")]
    requests: u8,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    simple_logger::init()?;
    let args: Args = Args::from_args();

    let stream = TcpStream::connect(args.addr);
    let timeout_time = Duration::from_secs(2);
    let stream = tokio::time::timeout(timeout_time, stream).await??;

    let (sender, connection) = h2::client::handshake(stream).await?;
    tokio::spawn(async { connection.await.expect("connection failed") });

    let mut handlers = Vec::with_capacity(args.requests as usize);
    for req in 1..=args.requests {
        let handler = tokio::spawn(send_request(req, sender.clone()));
        handlers.push(handler);
    }

    let join_all = futures::future::join_all(handlers).await;
    let times: Vec<_> = join_all
        .into_iter()
        .inspect(|join_handler| {
            if let Err(err) = join_handler {
                log::error!("{}", err)
            }
        })
        .filter_map(|join_handler| join_handler.ok())
        .inspect(|result| {
            if let Err(err) = result {
                log::error!("{}", err)
            }
        })
        .filter_map(|result| result.ok())
        .collect();

    let successful = times.len();
    let total: Duration = times.iter().sum();
    let min = times.iter().copied().min().unwrap_or_default();
    let max = times.iter().copied().max().unwrap_or_default();
    let avg = if successful != 0 {
        total / successful as u32
    } else {
        Duration::default()
    };

    log::info!(
        "Successful requests: {}/{}, total: {}ms, min: {}ms, max: {}ms, avg: {}ms",
        times.len(),
        args.requests,
        total.as_millis(),
        min.as_millis(),
        max.as_millis(),
        avg.as_millis()
    );

    Ok(())
}

async fn send_request(req: u8, mut sender: SendRequest<Bytes>) -> anyhow::Result<Duration> {
    let request = Request::post("http://127.0.0.1/")
        .version(Version::HTTP_2)
        .body(())?;

    let (response, _) = sender.send_request(request, true)?;
    let now = Instant::now();
    let response_result = response.await?;
    let server_response_time = now.elapsed();

    log::info!(
        "{req}: server response status code {}",
        response_result.status()
    );
    log::info!(
        "{req}: response time {}ms",
        server_response_time.as_millis()
    );

    Ok(server_response_time)
}
