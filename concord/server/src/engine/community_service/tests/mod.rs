use super::*;

fn server_id(value: &str) -> ServerId {
    ServerId::from_stored(value).unwrap()
}

fn channel_id(value: &str) -> ChannelId {
    ChannelId::from_stored(value).unwrap()
}

mod authorization;
mod behavior;
mod membership;
mod recovery;
mod validation;
