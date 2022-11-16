use bytes::Bytes;
use h2::server::SendResponse;
use std::net::SocketAddrV4;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use structopt::StructOpt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, Semaphore};

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
        help = "Макс. количество одновременных запросов"
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

    let (watch_tx, watch_rx) = watch::channel(());
    let (mpsc_tx, mut mpsc_rx) = mpsc::channel(1);

    let queue = Arc::new(Semaphore::new(args.limit));
    let data = Arc::new(Mutex::new(Vec::new()));

    let mut handlers = Vec::new();
    let listener = TcpListener::bind(args.addr).await?;
    loop {
        tokio::select! {
            connection = listener.accept() => {
                let (stream, _) = connection?;

                let handler = tokio::spawn(handle_client(
                    stream,
                    Arc::clone(&data),
                    Arc::clone(&queue),
                    watch_rx.clone(),
                    mpsc_tx.clone(),
                ));
                handlers.push(handler);
            },
            signal_result = tokio::signal::ctrl_c() => {
                signal_result?;
                log::info!("Ctrl-c received");

                let _ = watch_tx.send(()); // Игнорируем ошибку
                break;
            },
        }
    }

    drop(mpsc_tx); // Дропаем, чтобы mpsc_rx работал корректно.

    let timeout = Duration::from_secs(args.shutdown_time_sec);
    if tokio::time::timeout(timeout, mpsc_rx.recv()).await.is_err() {
        log::error!("Tasks not finished");

        // Если mpsc_rx.recv не выполнился, значит какие-то такси все еще работают. Их отменяем.
        // Такую технику подсмотрел здесь https://tokio.rs/tokio/topics/shutdown
        handlers
            .iter()
            .filter(|handler| !handler.is_finished())
            .for_each(|handler| handler.abort());
    };

    let join_all = futures::future::join_all(handlers).await;
    let blocked = join_all
        .into_iter()
        .fold(0, |counter, handler| match handler {
            Ok(Ok(_)) => counter,
            _ => counter + 1, // Либо отмена таски, либо ошибка при обработке запросов
        });

    print_total_statistic(data, blocked);

    Ok(())
}

pub async fn handle_client(
    stream: TcpStream,
    data: Arc<Mutex<Vec<ClientStatistic>>>,
    queue: Arc<Semaphore>,
    mut stop_signal: watch::Receiver<()>,
    _shutdowned: mpsc::Sender<()>,
) -> anyhow::Result<()> {
    use h2::server;
    use rand::distributions::Distribution;
    use rand::distributions::Uniform;
    use rand::rngs::SmallRng;
    use rand::SeedableRng;

    let _permit = queue.acquire_owned().await?;

    let between = Uniform::new_inclusive(100, 500);
    let mut rng = SmallRng::from_entropy();

    let addr = stream.peer_addr()?;
    let mut connection = server::handshake(stream).await?;
    log::info!("{}: connected", addr);

    let mut handlers = Vec::new();
    while let Some(request) = connection.accept().await {
        let (_, responder) = request?;

        let routine_time = Duration::from_millis(between.sample(&mut rng));
        let handler = tokio::spawn(client_routine(responder, routine_time));
        handlers.push(handler);

        if let Ok(true) = stop_signal.has_changed() {
            // Можно сделать так же через tokio::select и stop_signal.changed().await
            connection.graceful_shutdown();
            let _ = stop_signal.borrow_and_update();
        }
    }

    // Если произошла какая-то ошибка, то мы не смогли до конца обработать запросы клиентов
    // В таком случае возвращаем ошибку, т.к. данные не отображают полную статистику
    let try_join_all = futures::future::try_join_all(handlers).await?;
    let statistic = try_join_all.into_iter().try_fold(
        ClientStatistic::default(),
        |mut total, routine_result| {
            let time = routine_result?;
            total.update(time);
            Ok::<_, anyhow::Error>(total)
        },
    )?;

    let mut guard = match data.lock() {
        Ok(g) => g,
        Err(err) => err.into_inner(),
    };
    guard.push(statistic);

    log::info!("{}: disconnected", addr);
    Ok(())
}

pub async fn client_routine(
    mut responder: SendResponse<Bytes>,
    routine_time: Duration,
) -> anyhow::Result<Duration> {
    let response = http::Response::builder()
        .status(http::StatusCode::OK)
        .body(())?;

    tokio::time::sleep(routine_time).await;

    let _ = responder.send_response(response, true)?;
    Ok(routine_time)
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
