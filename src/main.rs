use std::borrow::Cow;
use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::CloseFrame;
use axum::extract::{ConnectInfo, State};
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::Path,
    response::IntoResponse,
    response::Response,
    routing::get,
    Router,
};
use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use include_dir::{include_dir, Dir};
use minijinja::{context, Environment};
use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::fs::{read_dir, File};
use tokio::io::AsyncReadExt; // for read_to_end()

static STATIC_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/public");

struct AppState {
    env: Environment<'static>,
}

#[tokio::main]
async fn main() {
    // initialize tracing
    // tracing_subscriber::fmt::init();
    let layout = STATIC_DIR.get_file("index.html").unwrap();
    let mut env = Environment::new();
    env.add_template("layout", layout.contents_utf8().unwrap_or(""))
        .unwrap();

    let app_state = Arc::new(AppState { env });

    let app = Router::new()
        .route("/", get(root))
        .route("/ws", get(ws_handler))
        .route("/*path", get(path_handler))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    println!("Server listening on {}!", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn root() -> impl IntoResponse {
    let mut result = match read_dir(".").await {
        Ok(entries) => entries,
        Err(_e) => return "Oops something has gone wrong".to_string(),
    };

    let mut listing = "".to_owned();

    while let Some(child) = result.next_entry().await.unwrap_or(None) {
        listing.push_str(
            child.path().to_str().unwrap_or(""), // .strip_prefix("./www/")
                                                 // .unwrap_or(""),
        );
        listing.push_str("\n")
    }

    String::from(&listing)
}

async fn path_handler(Path(path): Path<String>, State(state): State<Arc<AppState>>) -> Response {
    let template = state.env.get_template("layout").unwrap();
    let script = STATIC_DIR
        .get_file("index.js")
        .unwrap()
        .contents_utf8()
        .unwrap_or("");

    let file_path = format!("./{}", path);
    let mut file = match File::open(file_path).await {
        Ok(file) => file,
        Err(_e) => return "File not found".into_response(),
    };

    let mut contents = vec![];
    let result = file.read_to_end(&mut contents).await;

    if result.is_err() {
        return "Something wen't wrong trying to read the file".into_response();
    }

    let result = markdown::to_html_with_options(
        &String::from_utf8(contents).unwrap_or("".to_string()),
        &markdown::Options::gfm(),
    )
    .unwrap();

    let rendered = template
        .render(context! {
            markdown => result,
            syntax_js => script,
        })
        .unwrap();

    Response::builder()
        .status(200)
        .header("content-type", "text/html")
        .body(Body::from(rendered))
        .expect("This should not fail")
}

/// The handler for the HTTP request (this gets called when the HTTP GET lands at the start
/// of websocket negotiation). After this completes, the actual switching from HTTP to
/// websocket protocol will occur.
/// This is the last point where we can extract TCP/IP metadata such as IP address of the client
/// as well as things from HTTP headers such as user-agent of the browser etc.
async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
    //     user_agent.to_string()
    // } else {
    //     String::from("Unknown browser")
    // };
    // println!("`{user_agent}` at {addr} connected.");
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.
    println!("Starting websocket upgrade!");
    ws.on_upgrade(move |socket| handle_socket(socket, addr))
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket(mut socket: WebSocket, who: SocketAddr) {
    // send a ping (unsupported by some browsers) just to kick things off and get a response
    if socket.send(Message::Ping(vec![1, 2, 3])).await.is_ok() {
        println!("Pinged {who}...");
    } else {
        println!("Could not send ping {who}!");
        // no Error here since the only thing we can do is to close the connection.
        // If we can not send messages, there is no way to salvage the statemachine anyway.
        return;
    }

    let (mut sender, mut receiver) = socket.split();

    // Spawn a task that will push several messages to the client (does not matter what client does)
    let mut send_task = tokio::spawn(async move {
        let (mut watcher, mut rx) = async_watcher().unwrap();

        // Add a path to be watched. All files and directories at that path and
        // below will be monitored for changes.
        watcher
            .watch(".".as_ref(), RecursiveMode::Recursive)
            .unwrap();

        while let Some(res) = rx.next().await {
            match res {
                Ok(event) => {
                    // println!("changed: {:?}", event);
                    match event.kind {
                        EventKind::Modify(ModifyKind::Data(_)) => {
                            let paths = event.paths.first().unwrap().to_str().unwrap_or("");
                            if sender
                                .send(Message::Text(format!("File changed {paths}")))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                Err(e) => println!("watch error: {:?}", e),
            }
        }
    });

    // This second task will receive messages from client and print them on server console
    let mut recv_task = tokio::spawn(async move {
        let mut cnt = 0;
        while let Some(Ok(msg)) = receiver.next().await {
            cnt += 1;
            // print message and break if instructed to do so
            if process_message(msg, who).is_break() {
                break;
            }
        }
        cnt
    });

    // If any one of the tasks exit, abort the other.
    tokio::select! {
        rv_a = (&mut send_task) => {
            match rv_a {
                Ok(_) => println!("Some messages sent to {who}"),
                Err(a) => println!("Error sending messages {a:?}")
            }
            recv_task.abort();
        },
        rv_b = (&mut recv_task) => {
            match rv_b {
                Ok(b) => println!("Received {b} messages"),
                Err(b) => println!("Error receiving messages {b:?}")
            }
            send_task.abort();
        }
    }

    // returning from the handler closes the websocket connection
    println!("Websocket context {who} destroyed");
}

fn process_message(msg: Message, who: SocketAddr) -> ControlFlow<(), ()> {
    match msg {
        Message::Text(t) => {
            eprintln!(">>> {who} sent str: {t:?}");
        }
        Message::Binary(d) => {
            eprintln!(">>> {} sent {} bytes: {:?}", who, d.len(), d);
        }
        Message::Close(c) => {
            if let Some(cf) = c {
                eprintln!(
                    ">>> {} sent close with code {} and reason `{}`",
                    who, cf.code, cf.reason
                );
            } else {
                eprintln!(">>> {who} somehow sent close message without CloseFrame");
            }
            return ControlFlow::Break(());
        }

        Message::Pong(v) => {
            eprintln!(">>> {who} sent pong with {v:?}");
        }
        // You should never need to manually handle Message::Ping, as axum's websocket library
        // will do so for you automagically by replying with Pong and copying the v according to
        // spec. But if you need the contents of the pings you can see them here.
        Message::Ping(v) => {
            eprintln!(">>> {who} sent ping with {v:?}");
        }
    }
    ControlFlow::Continue(())
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (mut tx, rx) = channel(1);

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let watcher = RecommendedWatcher::new(
        move |res| {
            futures::executor::block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}
