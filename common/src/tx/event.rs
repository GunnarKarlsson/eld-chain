use abci::types::EventAttribute;

pub fn create_event_attribute(key: String, value: String) -> EventAttribute {
    let e = EventAttribute {
        key: key.as_bytes().to_vec(),
        value: value.as_bytes().to_vec(),
        index: true,
    };
    e
}
