use std::net::SocketAddrV4;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use structopt::StructOpt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Semaphore};

#[derive(Debug)]
pub struct ClientStatistic {
    pub requests: usize,
    pub total_time: Duration,
    pub min_time: Duration,
    pub max_time: Duration,
    pub avg_time: Duration,
}

impl Default for ClientStatistic {
    #[inline]
    fn default() -> Self {
        Self {
            requests: 0,
            total_time: Duration::ZERO,
            min_time: Duration::MAX,
            max_time: Duration::ZERO,
            avg_time: Duration::ZERO,
        }
    }
}

impl ClientStatistic {
    pub fn update(&mut self, from: Duration) {
        self.requests += 1;
        self.total_time += from;
        self.min_time = self.min_time.min(from);
        self.max_time = self.max_time.max(from);
        self.avg_time = self.total_time / self.requests as u32;
    }
}

#[derive(Debug, StructOpt)]
struct Args {
    #[structopt(short, long, default_value = "127.0.0.1:8080", help = "Адрес сервера")]
    addr: SocketAddrV4,
    #[structopt(
        short,
        long,
        default_value = "5",
        help = "Мас. количество одновременных запросов"
    )]
    limit: usize,
    #[structopt(
        short,
        long,
        default_value = "10",
        help = "Время ожидания завершения сервера (в сек.)"
    )]
    shutdown_time_sec: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    simple_logger::init()?;
    let args: Args = Args::from_args();

    let (tx, rx) = watch::channel(());
    let queue = Arc::new(Semaphore::new(args.limit));
    let data = Arc::new(Mutex::new(Vec::new()));

    let mut handlers = Vec::new();
    let listener = TcpListener::bind(args.addr).await?;
    loop {
        tokio::select! {
            connection = listener.accept() => {
                let (stream, _) = connection?;

                let join_handler = tokio::spawn(handle_client(stream, Arc::clone(&data), Arc::clone(&queue), rx.clone()));
                handlers.push(join_handler);
            },
            signal_result = tokio::signal::ctrl_c() => {
                signal_result?;
                log::info!("Ctrl-c received");

                let _ = tx.send(()); // Игнорируем ошибку
                break;
            },
        }
    }

    let timeout = Duration::from_secs(args.shutdown_time_sec);
    let join_all_handler = futures::future::join_all(handlers);

    let blocked = match tokio::time::timeout(timeout, join_all_handler).await {
        Ok(_) => 0, // Если получилось сджойнить все таски, значит все запросы были обработаны
        Err(_) => args.limit - queue.available_permits(),
    };

    print_total_statistic(data, blocked);

    Ok(())
}

pub async fn handle_client(
    stream: TcpStream,
    data: Arc<Mutex<Vec<ClientStatistic>>>,
    queue: Arc<Semaphore>,
    mut stop_signal: watch::Receiver<()>,
) -> anyhow::Result<()> {
    use h2::server;
    use rand::distributions::Distribution;
    use rand::distributions::Uniform;
    use rand::rngs::SmallRng;
    use rand::SeedableRng;

    let _permit = queue.acquire_owned().await?;

    let mut statistic = ClientStatistic::default();

    let between = Uniform::new_inclusive(100, 500);
    let mut rng = SmallRng::from_entropy();

    let addr = stream.peer_addr()?;
    let mut connection = server::handshake(stream).await?;
    log::info!("Client {} connected", addr);

    while let Some(request) = connection.accept().await {
        let (_, mut responder) = request?;

        if let Ok(true) = stop_signal.has_changed() {
            // Можно сделать так же через tokio::select и stop_signal.changed().await
            connection.graceful_shutdown();
            let _ = stop_signal.borrow_and_update();
        }

        let routine_time = Duration::from_millis(between.sample(&mut rng));
        tokio::time::sleep(routine_time).await;

        let response = http::Response::builder()
            .status(http::StatusCode::OK)
            .body(())?;

        let _ = responder.send_response(response, true)?;
        statistic.update(routine_time);
    }

    let mut guard = match data.lock() {
        Ok(g) => g,
        Err(err) => err.into_inner(),
    };
    guard.push(statistic);

    log::info!("Client {} disconnected", addr);
    Ok(())
}

fn print_total_statistic(data: Arc<Mutex<Vec<ClientStatistic>>>, blocked: usize) {
    let statistics = match data.lock() {
        Ok(g) => g,
        Err(err) => err.into_inner(),
    };

    let clients_count = statistics.len();
    let min_time = statistics
        .iter()
        .map(|s| s.min_time)
        .min()
        .unwrap_or_default();
    let max_time = statistics
        .iter()
        .map(|s| s.max_time)
        .max()
        .unwrap_or_default();

    let total_time: Duration = statistics.iter().map(|s| s.total_time).sum();
    let requests_count: u32 = statistics.iter().map(|s| s.requests as u32).sum();

    let avg_time = if requests_count != 0 {
        total_time / requests_count
    } else {
        Duration::default()
    };

    log::info!(
        "Total statistics - accepted clients: {}, blocked clients: {}, requests: {}, \
        total time: {}ms, min: {}ms, max: {}ms, avg: {}ms",
        clients_count,
        blocked,
        requests_count,
        total_time.as_millis(),
        min_time.as_millis(),
        max_time.as_millis(),
        avg_time.as_millis()
    );
}
