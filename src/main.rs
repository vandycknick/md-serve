use axum::body::Body;
use axum::{extract::Path, response::IntoResponse, response::Response, routing::get, Router};
use tokio::fs::{read_dir, File};
use tokio::io::AsyncReadExt; // for read_to_end()

#[tokio::main]
async fn main() {
    // initialize tracing
    // tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/", get(root))
        .route("/*path", get(path_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn root() -> impl IntoResponse {
    let mut result = match read_dir("./www").await {
        Ok(entries) => entries,
        Err(_e) => return "Oops something has gone wrong".to_string(),
    };

    let mut listing = "".to_owned();

    while let Some(child) = result.next_entry().await.unwrap_or(None) {
        listing.push_str(
            child
                .path()
                .to_str()
                .unwrap_or("")
                .strip_prefix("./www/")
                .unwrap_or(""),
        );
        listing.push_str("\n")
    }

    String::from(&listing)
}

// basic handler that responds with a static string
async fn path_handler(Path(path): Path<String>) -> Response {
    let file_path = format!("./www/{}", path);
    let mut file = match File::open(file_path).await {
        Ok(file) => file,
        Err(_e) => return "File not found".into_response(),
    };

    let mut contents = vec![];
    let result = file.read_to_end(&mut contents).await;

    if result.is_err() {
        return "Something wen't wrong trying to read the file".into_response();
    }

    println!("len = {}", contents.len());
    // String::from_utf8(contents)
    //     .expect("TODO implement a check to see if this is a utf8 readable file");
    //

    let result = markdown::to_html(&String::from_utf8(contents).unwrap_or("".to_string()));

    Response::builder()
        .status(200)
        .header("content-type", "text/html")
        .body(Body::from(result))
        .expect("This should not fail")
}
