use serde::Serialize;

#[derive(Serialize)]
struct Event {
    game: String,
    event: String,
    data: Data,
}

#[derive(Serialize)]
struct Data{
    value: u8
}

impl Event {
    fn new(game: String, event: String, data: Data) -> Self {
        Event { game, event, data }
    }
}

struct RegisterGame {
    game: String,
    game_display_name: String,
    developer: String
}

#[derive(Serialize)]
struct RegisterEvent {
    game: String,
    event: String,
    min_value: Option<u8>,
    max_value: Option<u8>,
    icon_id: Option<u8>,
    value_optional: bool
}

fn main() {
   let register = RegisterEvent {
        game: "game".to_string(),
        event: "event".to_string(),
        min_value: Some(0),
        max_value: Some(100),
        icon_id: Some(1),
        value_optional: false
    };
   
   println!("{}", serde_json::to_string(&register).unwrap())
}