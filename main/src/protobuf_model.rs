use prost;
use prost::{Enumeration, Message};

#[allow(dead_code)]
#[derive(Message)]
pub struct LogEntry {
    #[prost(int32, tag = "1")]
    pub message_type: i32,

    #[prost(string, tag = "2")]
    pub host: String,

    #[prost(int64, tag = "3")]
    pub micro_second_timestamp: i64,

    #[prost(enumeration = "LogLevel", tag = "4")]
    pub log_level: i32,

    #[prost(string, tag = "5")]
    pub target: String,

    #[prost(string, tag = "6")]
    pub args: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Enumeration)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}
