use crate::model::SessionNode;

pub trait Provider {
    fn is_available(&self) -> bool;
    fn load_sessions(&self, backstop: u64) -> Vec<SessionNode>;
}
