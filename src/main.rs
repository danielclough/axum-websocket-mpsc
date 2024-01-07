use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};

use futures::{sink::SinkExt, stream::StreamExt};
use tokio::sync::{broadcast, mpsc};

struct AppState {
    // We require unique usernames. This tracks which usernames have been taken.
    user_set: Mutex<HashSet<String>>,
    // Channel used to send messages to all connected clients.
    broadcast_sender: broadcast::Sender<String>,
}

impl Default for AppState {
    fn default() -> Self {
        let (broadcast_sender, _) = broadcast::channel(16);
        let user_set = Mutex::new(HashSet::new());
        Self { broadcast_sender, user_set }
    }
}

use std::{thread::sleep, time::Duration};

#[tokio::main]
async fn main() {
    // Set up application state for use with with_state().
    let app_state = Arc::new(AppState::default());

    let app = Router::new()
        .route("/", get(index))
        .route("/websocket", get(websocket_handler))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
    .await
    .unwrap();
    println!("listening on {:?}", listener);
    axum::serve(listener, app).await.unwrap();
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| websocket(socket, state))
}

// This function deals with a single websocket connection, i.e., a single
// connected client / user, for which we will spawn two independent tasks (for
// receiving / sending chat messages).
async fn websocket(stream: WebSocket, state: Arc<AppState>) {
    // By splitting, we can send and receive at the same time.
    let (mut sender_sink, mut receiver_stream) = stream.split();
    let (sender_mpsc, mut receiver_mpsc) = mpsc::channel::<String>(16);

    tokio::spawn(async move {
        while let Some(message) = receiver_mpsc.recv().await {
            if sender_sink.send(message.into()).await.is_err() {
                break;
            }
        }
    });
    // Username gets set in the receive loop, if it's valid.
    let mut username = String::new();
    // Loop until a text message is found.
    while let Some(Ok(message)) = receiver_stream.next().await {
        if let Message::Text(name) = message {
            // If username that is sent by client is not taken, fill username string.
            check_username(&state, &mut username, &name);

            // If not empty we want to quit the loop else we want to quit function.
            if !username.is_empty() {
                println!("Username Empty");
                break;
            } else {
                // Only send our client that username is taken.
                let _ = sender_mpsc
                    .send(String::from("Username already taken."))
                    .await;

                return;
            }
        }
    }

    // We subscribe *before* sending the "joined" message, so that we will also
    // display it to our client.
    let mut receiver_subscribed = state.broadcast_sender.subscribe();

    // Now send the "joined" message to all subscribers.
    let join_msg = format!("{username} joined.");
    println!("{join_msg}");
    let _ = state.broadcast_sender.send(join_msg);

    // Clone things we want to pass (move) to the receiving task.
    // forward to mpsc from receiver_subscribed
    let sender_clone = sender_mpsc.clone();


    // create new thread for processing messages.
    // This response happens immediately
    let mut send_join_handle = tokio::spawn(async move {
        while let Ok(msg_out) = receiver_subscribed.recv().await {            

            if sender_clone
                .send(format!("{msg_out}"))
                .await
                .is_err()
            {
                // break on err
                println!("sender_clone error");
                break;
            }
        }
    });

    // forward user messages to broadcast_sender_clone
    let broadcast_sender_clone = state.broadcast_sender.clone();
    let sender_mpsc_clone = sender_mpsc.clone();
    let name_cloned = username.clone();
    let name_cloned1 = username.clone();
    
    // create new thread
    let mut recv_join_handle = tokio::spawn(async move {
        while let Some(Ok(Message::Text(msg_in))) = receiver_stream.next().await {

            // This gets sent into send_join_handle and returned immediately.
            // Using to tell user that message is being processed, only sent to original sender.
            let send_immediately = format!("{},\nYour results will be ready in a moment.",name_cloned);
            if sender_mpsc_clone
                .send(String::from(send_immediately))
                .await
                .is_err()
            {
                // break on err
                break;
            }

            // Clone msg_in in order to return original message to both clients.
            let msg_in_cloned = msg_in.clone();

            // This allows long running tasks to be sent after logging original message.
            // sleep 3 sec
            let work_done = tokio::task::spawn_blocking(move || {
                sleep(Duration::from_secs(3));
                format!("Now 3 seconds after \"{}\" started processing on a blocking thread!",  msg_in)
            })
            .await
            .unwrap();
        
            let bot_response = format!("{name_cloned1}: {msg_in_cloned}\nBot: {work_done}");

            let _ = broadcast_sender_clone.send(bot_response);
            
        }
    });

    // If any one of the tasks run to completion, we abort the other.
    tokio::select! {
        _ = (&mut send_join_handle) => recv_join_handle.abort(),
        _ = (&mut recv_join_handle) => send_join_handle.abort(),
    };

    // Send "user left" message (similar to "joined" above).
    let msg = format!("{username} left.");
    let _ = state.broadcast_sender.send(msg);

    // Remove username from map so new clients can take it again.
    state.user_set.lock().unwrap().remove(&username);
}

// Include utf-8 file at **compile** time.
async fn index() -> Html<&'static str> {
    Html(std::include_str!("../chat.html"))
}


fn check_username(state: &AppState, string: &mut String, name: &str) {
    let mut user_set = state.user_set.lock().unwrap();

    if !user_set.contains(name) {
        user_set.insert(name.to_owned());

        string.push_str(name);
    }
}
