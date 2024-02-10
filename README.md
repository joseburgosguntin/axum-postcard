# 📬 postcard axum extractor and response using serde

## Example

### Request

```rust
use axum::{extract, routing::post, Router};
use serde::Deserialize;
use axum_postcard::Postcard;

#[derive(Deserialize)]
struct CreateUser {
    email: String,
    password: String,
}

async fn create_user(
    Postcard(payload): Postcard<CreateUser>
) {
    // payload is a `CreateUser`
    todo!()
}
```

### Response

```rust
use axum::{extract::Path, routing::get, Router};
use serde::Serialize;
use axum_postcard::Postcard;

#[derive(Serialize)]
struct User {
    id: u32,
    username: String,
}

async fn get_user(
    Path(user_id) : Path<u32>
) -> Postcard<User> {
    let user = find_user(user_id).await;
    Postcard(user)
}


async fn find_user(user_id: u32) -> User {
    todo!()
}
```
