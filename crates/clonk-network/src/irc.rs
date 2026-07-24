//! Legacy plaintext IRC client used by the classic community-chat screen.
//!
//! The protocol reducer is deliberately independent from the socket worker so
//! that every state transition can be checked without an external IRC server.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    Arc, Mutex, MutexGuard,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use thiserror::Error;

pub const IRC_DEFAULT_PORT: u16 = 6666;
pub const IRC_MAX_LOG_LENGTH: usize = 300_000;
pub const IRC_MAX_READ_LOG_LENGTH: usize = 1_000;

const IRC_READ_POLL_INTERVAL: Duration = Duration::from_millis(20);
const IRC_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IRC_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const IRC_CTCP_ENGINE_VERSION: &str = "4.9.11.0 [362] ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IrcMessageType {
    Server,
    Status,
    Message,
    Notice,
    Action,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrcMessage {
    pub timestamp: SystemTime,
    pub message_type: IrcMessageType,
    pub source: Vec<u8>,
    pub target: Vec<u8>,
    pub data: Vec<u8>,
}

impl IrcMessage {
    pub fn is_channel(&self) -> bool {
        self.target
            .first()
            .is_some_and(|byte| matches!(byte, b'#' | b'+'))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrcUser {
    pub prefix: Vec<u8>,
    pub name: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrcChannel {
    pub name: Vec<u8>,
    pub topic: Vec<u8>,
    pub users: Vec<IrcUser>,
    pub receiving_users: bool,
}

impl IrcChannel {
    fn new(name: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            topic: Vec::new(),
            users: Vec::new(),
            receiving_users: false,
        }
    }

    pub fn user(&self, name: impl AsRef<[u8]>) -> Option<&IrcUser> {
        let name = c_string_bytes(name.as_ref());
        self.users
            .iter()
            .find(|user| c_string_bytes(&user.name) == name)
    }

    fn add_user(&mut self, name: &[u8]) -> &mut IrcUser {
        let name = c_string_bytes(name);
        if let Some(index) = self
            .users
            .iter()
            .position(|user| c_string_bytes(&user.name) == name)
        {
            return &mut self.users[index];
        }
        self.users.insert(
            0,
            IrcUser {
                prefix: Vec::new(),
                name: name.to_vec(),
            },
        );
        &mut self.users[0]
    }

    fn remove_user(&mut self, name: &[u8]) -> bool {
        let name = c_string_bytes(name);
        let Some(index) = self
            .users
            .iter()
            .position(|user| c_string_bytes(&user.name) == name)
        else {
            return false;
        };
        self.users.remove(index);
        true
    }

    fn receive_users(&mut self, names: &[u8], prefix_map: &[u8]) {
        let prefix_chars = prefix_map
            .iter()
            .position(|byte| *byte == b')')
            .map_or(&[][..], |closing| &prefix_map[closing + 1..]);
        if !self.receiving_users {
            self.users.clear();
        }

        let mut parameters = Some(names);
        while parameters.is_some_and(|remaining| !remaining.is_empty()) {
            let prefixed_name = extract_parameter(&mut parameters);
            let name_start = prefixed_name
                .iter()
                .position(|byte| !prefix_chars.contains(byte))
                .unwrap_or(prefixed_name.len());
            let (prefix, name) = prefixed_name.split_at(name_start);
            // The C++ code walks beyond the terminator for a prefix-only token.
            // Safely ignore that malformed token instead.
            if name.is_empty() {
                continue;
            }
            self.add_user(name).prefix = prefix.to_vec();
        }
        self.receiving_users = true;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IrcConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrcConnectConfig {
    pub server: String,
    pub nick: Vec<u8>,
    pub real_name: Vec<u8>,
    pub password: Option<Vec<u8>>,
    pub auto_join: Option<Vec<u8>>,
    /// Payload following the CTCP `VERSION` tag, without CTCP delimiters.
    pub ctcp_version: Vec<u8>,
}

impl IrcConnectConfig {
    pub fn new(server: impl Into<String>, nick: Vec<u8>, real_name: Vec<u8>) -> Self {
        Self {
            server: server.into(),
            nick,
            real_name,
            password: None,
            auto_join: None,
            ctcp_version: format!("Clonk Rust:{}:{}", IRC_CTCP_ENGINE_VERSION, c4_os_tag())
                .into_bytes(),
        }
    }
}

const fn c4_os_tag() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "mac"
    }
    #[cfg(all(target_os = "linux", target_pointer_width = "32"))]
    {
        "linux"
    }
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    {
        "linux64"
    }
    #[cfg(all(target_os = "windows", target_pointer_width = "32"))]
    {
        "win32"
    }
    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    {
        "win64"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        std::env::consts::OS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrcCommand {
    /// Send one complete IRC payload without the trailing CRLF.
    Raw(Vec<u8>),
    Send {
        command: Vec<u8>,
        parameters: Option<Vec<u8>>,
    },
    Quit {
        reason: Vec<u8>,
    },
    Join {
        channel: Vec<u8>,
    },
    Part {
        channel: Vec<u8>,
    },
    Message {
        target: Vec<u8>,
        text: Vec<u8>,
    },
    Notice {
        target: Vec<u8>,
        text: Vec<u8>,
    },
    Action {
        target: Vec<u8>,
        text: Vec<u8>,
    },
    ChangeNick {
        nick: Vec<u8>,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IrcReduceResult {
    pub outbound: Vec<Vec<u8>>,
    /// One notification per appended message, plus the NAMES-end notification.
    pub notifications: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IrcClientSnapshot {
    pub connection_state: IrcConnectionState,
    pub nick: Vec<u8>,
    pub prefixes: Vec<u8>,
    pub channels: Vec<IrcChannel>,
    pub messages: Vec<IrcMessage>,
    pub unread_index: usize,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct IrcClientState {
    connection_state: IrcConnectionState,
    nick: Vec<u8>,
    real_name: Vec<u8>,
    password: Option<Vec<u8>>,
    auto_join: Option<Vec<u8>>,
    ctcp_version: Vec<u8>,
    prefixes: Vec<u8>,
    channels: Vec<IrcChannel>,
    messages: VecDeque<IrcMessage>,
    unread_index: usize,
    max_log_length: usize,
    max_read_log_length: usize,
    last_error: Option<String>,
}

impl Default for IrcClientState {
    fn default() -> Self {
        Self::new()
    }
}

impl IrcClientState {
    pub fn new() -> Self {
        Self::with_log_limits(IRC_MAX_LOG_LENGTH, IRC_MAX_READ_LOG_LENGTH)
    }

    fn with_log_limits(max_log_length: usize, max_read_log_length: usize) -> Self {
        Self {
            connection_state: IrcConnectionState::Disconnected,
            nick: Vec::new(),
            real_name: Vec::new(),
            password: None,
            auto_join: None,
            ctcp_version: Vec::new(),
            prefixes: b"(ov)@+".to_vec(),
            channels: Vec::new(),
            messages: VecDeque::new(),
            unread_index: 0,
            max_log_length,
            max_read_log_length,
            last_error: None,
        }
    }

    pub fn connection_state(&self) -> IrcConnectionState {
        self.connection_state
    }

    pub fn is_active(&self) -> bool {
        self.connection_state != IrcConnectionState::Disconnected
    }

    pub fn is_connected(&self) -> bool {
        self.connection_state == IrcConnectionState::Connected
    }

    pub fn user_name(&self) -> &[u8] {
        &self.nick
    }

    pub fn prefixes(&self) -> &[u8] {
        &self.prefixes
    }

    pub fn channels(&self) -> &[IrcChannel] {
        &self.channels
    }

    pub fn channel(&self, name: impl AsRef<[u8]>) -> Option<&IrcChannel> {
        let name = name.as_ref();
        self.channels
            .iter()
            .find(|channel| irc_eq(&channel.name, name))
    }

    pub fn messages(&self) -> impl Iterator<Item = &IrcMessage> {
        self.messages.iter()
    }

    pub fn unread_messages(&self) -> impl Iterator<Item = &IrcMessage> {
        self.messages.iter().skip(self.unread_index)
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn snapshot(&self) -> IrcClientSnapshot {
        IrcClientSnapshot {
            connection_state: self.connection_state,
            nick: self.nick.clone(),
            prefixes: self.prefixes.clone(),
            channels: self.channels.clone(),
            messages: self.messages.iter().cloned().collect(),
            unread_index: self.unread_index,
            last_error: self.last_error.clone(),
        }
    }

    fn snapshot_and_mark_message_log_read(&mut self) -> IrcClientSnapshot {
        let snapshot = self.snapshot();
        self.mark_message_log_read();
        snapshot
    }

    pub fn begin_connect(&mut self, mut config: IrcConnectConfig) {
        if self.is_active() {
            self.close();
        }
        truncate_at_nul(&mut config.nick);
        truncate_at_nul(&mut config.real_name);
        if let Some(password) = &mut config.password {
            truncate_at_nul(password);
            password.truncate(31);
        }
        if let Some(auto_join) = &mut config.auto_join {
            truncate_at_nul(auto_join);
        }
        truncate_at_nul(&mut config.ctcp_version);
        self.connection_state = IrcConnectionState::Connecting;
        self.nick = config.nick;
        self.real_name = config.real_name;
        self.password = config.password;
        self.auto_join = config.auto_join;
        self.ctcp_version = config.ctcp_version;
        self.prefixes = b"(ov)@+".to_vec();
        self.last_error = None;
    }

    pub fn on_tcp_connected(&mut self) -> Result<IrcReduceResult, IrcClientError> {
        if self.connection_state != IrcConnectionState::Connecting {
            return Err(IrcClientError::InvalidState(
                "TCP connection completed while IRC was not connecting".to_owned(),
            ));
        }
        self.connection_state = IrcConnectionState::Connected;
        let mut outbound = Vec::with_capacity(3);
        if let Some(password) = &self.password {
            outbound.push(format_command(b"PASS", Some(password)));
        }
        outbound.push(format_command(b"NICK", Some(&self.nick)));
        outbound.push(format_command(
            b"USER",
            Some(&join_bytes(&[b"clonk x x :", &self.real_name])),
        ));
        Ok(IrcReduceResult {
            outbound,
            notifications: 0,
        })
    }

    pub fn on_disconnected(&mut self, reason: impl Into<String>) -> IrcReduceResult {
        let reason = reason.into();
        self.connection_state = IrcConnectionState::Disconnected;
        self.last_error = Some(reason.clone());
        let target = self.nick.clone();
        let notifications = self.push_message(
            IrcMessageType::Status,
            b"",
            &target,
            &join_bytes(&[b"Disconnected from server (", reason.as_bytes(), b")."]),
        );
        IrcReduceResult {
            outbound: Vec::new(),
            notifications,
        }
    }

    pub fn close(&mut self) {
        self.connection_state = IrcConnectionState::Disconnected;
        self.channels.clear();
        self.clear_message_log();
        self.last_error = None;
    }

    pub fn clear_message_log(&mut self) {
        self.messages.clear();
        self.unread_index = 0;
    }

    pub fn mark_message_log_read(&mut self) {
        self.unread_index = self.messages.len();
        while self.messages.len() > self.max_read_log_length {
            self.pop_message();
        }
    }

    pub fn outgoing_line(&self, command: &IrcCommand) -> Result<Vec<u8>, IrcClientError> {
        if !self.is_connected() {
            return Err(IrcClientError::NotConnected);
        }
        Ok(match command {
            IrcCommand::Raw(line) => c_string_bytes(line).to_vec(),
            IrcCommand::Send {
                command,
                parameters,
            } => format_command(command, parameters.as_deref()),
            IrcCommand::Quit { reason } => {
                format_command(b"QUIT", Some(&join_bytes(&[b":", reason])))
            }
            IrcCommand::Join { channel } => format_command(b"JOIN", Some(channel)),
            IrcCommand::Part { channel } => format_command(b"PART", Some(channel)),
            IrcCommand::Message { target, text } => {
                format_command(b"PRIVMSG", Some(&join_bytes(&[target, b" :", text])))
            }
            IrcCommand::Notice { target, text } => {
                format_command(b"NOTICE", Some(&join_bytes(&[target, b" :", text])))
            }
            IrcCommand::Action { target, text } => format_command(
                b"PRIVMSG",
                Some(&join_bytes(&[target, b" :\x01ACTION ", text, b"\x01"])),
            ),
            IrcCommand::ChangeNick { nick } => format_command(b"NICK", Some(nick)),
        })
    }

    pub fn mark_outgoing_sent(&mut self, command: &IrcCommand) -> IrcReduceResult {
        let notifications = match command {
            IrcCommand::Message { target, text } => {
                let nick = self.nick.clone();
                self.push_message(IrcMessageType::Message, &nick, target, text)
            }
            IrcCommand::Notice { target, text } => {
                let nick = self.nick.clone();
                self.push_message(IrcMessageType::Notice, &nick, target, text)
            }
            IrcCommand::Action { target, text } => {
                let nick = self.nick.clone();
                self.push_message(IrcMessageType::Action, &nick, target, text)
            }
            IrcCommand::Quit { .. } => {
                self.close();
                0
            }
            _ => 0,
        };
        IrcReduceResult {
            outbound: Vec::new(),
            notifications,
        }
    }

    /// Apply a server line after its LF and optional CR have been removed.
    pub fn receive_line(&mut self, line: impl AsRef<[u8]>) -> IrcReduceResult {
        let Some(parsed) = ParsedLine::parse(line.as_ref()) else {
            return IrcReduceResult::default();
        };
        if parsed.command.len() == 3 && parsed.command.first().is_some_and(u8::is_ascii_digit) {
            let numeric = parsed
                .command
                .iter()
                .copied()
                .take_while(u8::is_ascii_digit)
                .fold(0_i32, |value, digit| value * 10 + i32::from(digit - b'0'));
            return self.receive_numeric(&parsed.prefix, numeric, &parsed.parameters);
        }
        self.receive_command(&parsed.prefix, &parsed.command, &parsed.parameters)
    }

    fn receive_command(
        &mut self,
        sender: &[u8],
        command: &[u8],
        raw_parameters: &[u8],
    ) -> IrcReduceResult {
        let mut result = IrcReduceResult::default();
        let sender_nick = sender
            .split(|byte| *byte == b'!')
            .next()
            .unwrap_or_default()
            .to_vec();

        if irc_eq(command, b"PING") {
            result
                .outbound
                .push(format_command(b"PONG", Some(raw_parameters)));
        }

        if irc_eq(command, b"NOTICE") || irc_eq(command, b"PRIVMSG") {
            let mut parameters = Some(raw_parameters);
            let target = extract_parameter(&mut parameters);
            let text = extract_parameter(&mut parameters);
            let message_result =
                self.receive_message(irc_eq(command, b"NOTICE"), sender, &target, &text);
            result.outbound.extend(message_result.outbound);
            result.notifications += message_result.notifications;
        }

        if irc_eq(command, b"JOIN") {
            let mut parameters = Some(raw_parameters);
            let channel = extract_parameter(&mut parameters);
            let channel_index = self.add_channel(&channel);
            self.channels[channel_index].add_user(&sender_nick);
            let text = if sender_nick == self.nick {
                join_bytes(&[b"You have joined channel ", &channel, b"."])
            } else {
                join_bytes(&[&sender_nick, b" has joined the channel."])
            };
            result.notifications +=
                self.push_message(IrcMessageType::Status, sender, &channel, &text);
        }

        if irc_eq(command, b"PART") {
            let mut parameters = Some(raw_parameters);
            let channel = extract_parameter(&mut parameters);
            let comment = extract_parameter(&mut parameters);
            let channel_index = self.add_channel(&channel);
            self.channels[channel_index].remove_user(&sender_nick);
            if sender_nick == self.nick {
                self.remove_channel(&channel);
                let nick = self.nick.clone();
                result.notifications += self.push_message(
                    IrcMessageType::Status,
                    sender,
                    &nick,
                    &join_bytes(&[b"You have left channel ", &channel, b" (", &comment, b")."]),
                );
            } else {
                result.notifications += self.push_message(
                    IrcMessageType::Status,
                    sender,
                    &channel,
                    &join_bytes(&[&sender_nick, b" has left the channel (", &comment, b")"]),
                );
            }
        }

        if irc_eq(command, b"KICK") {
            let mut parameters = Some(raw_parameters);
            let channel = extract_parameter(&mut parameters);
            let kicked = extract_parameter(&mut parameters);
            let comment = extract_parameter(&mut parameters);
            let channel_index = self.add_channel(&channel);
            self.channels[channel_index].remove_user(&kicked);
            if kicked == self.nick {
                self.remove_channel(&channel);
                let nick = self.nick.clone();
                result.notifications += self.push_message(
                    IrcMessageType::Status,
                    sender,
                    &nick,
                    &join_bytes(&[
                        b"You were kicked from channel ",
                        &channel,
                        b" (",
                        &comment,
                        b").",
                    ]),
                );
            } else {
                result.notifications += self.push_message(
                    IrcMessageType::Status,
                    sender,
                    &channel,
                    &join_bytes(&[&kicked, b" was kicked from the channel (", &comment, b")."]),
                );
            }
        }

        if irc_eq(command, b"QUIT") {
            let mut parameters = Some(raw_parameters);
            let comment = extract_parameter(&mut parameters);
            let text = join_bytes(&[&sender_nick, b" has disconnected (", &comment, b")."]);
            let affected = self
                .channels
                .iter()
                .enumerate()
                .filter_map(|(index, channel)| {
                    channel.user(&sender_nick).is_some().then_some(index)
                })
                .collect::<Vec<_>>();
            for index in affected {
                let channel = self.channels[index].name.clone();
                self.channels[index].remove_user(&sender_nick);
                result.notifications +=
                    self.push_message(IrcMessageType::Status, sender, &channel, &text);
            }
        }

        if irc_eq(command, b"TOPIC") {
            let mut parameters = Some(raw_parameters);
            let channel = extract_parameter(&mut parameters);
            let topic = extract_parameter(&mut parameters);
            let channel_index = self.add_channel(&channel);
            self.channels[channel_index].topic.clone_from(&topic);
            result.notifications += self.push_message(
                IrcMessageType::Status,
                sender,
                &channel,
                &join_bytes(&[&sender_nick, b" changes the topic to: ", &topic]),
            );
        }

        if irc_eq(command, b"MODE") {
            let mut parameters = Some(raw_parameters);
            let channel = extract_parameter(&mut parameters);
            let flags = extract_parameter(&mut parameters);
            let what = extract_parameter(&mut parameters);
            if self.channel(&channel).is_some() {
                result
                    .outbound
                    .push(format_command(b"NAMES", Some(&channel)));
            }
            result.notifications += self.push_message(
                IrcMessageType::Status,
                sender,
                &channel,
                &join_bytes(&[&sender_nick, b" sets mode ", &flags, b" ", &what]),
            );
        }

        if irc_eq(command, b"ERROR") {
            let mut parameters = Some(raw_parameters);
            let message = extract_parameter(&mut parameters);
            let nick = self.nick.clone();
            result.notifications +=
                self.push_message(IrcMessageType::Server, sender, &nick, &message);
        }

        if irc_eq(command, b"NICK") {
            let mut parameters = Some(raw_parameters);
            let new_nick = extract_parameter(&mut parameters);
            let text = join_bytes(&[&sender_nick, b" is now known as ", &new_nick]);
            let affected = self
                .channels
                .iter()
                .enumerate()
                .filter_map(|(index, channel)| {
                    channel.user(&sender_nick).is_some().then_some(index)
                })
                .collect::<Vec<_>>();
            for index in affected {
                let channel = self.channels[index].name.clone();
                self.channels[index].remove_user(&sender_nick);
                self.channels[index].add_user(&new_nick);
                result.notifications +=
                    self.push_message(IrcMessageType::Status, sender, &channel, &text);
            }
            if sender_nick == self.nick {
                self.nick = new_nick;
            }
        }

        result
    }

    fn receive_numeric(
        &mut self,
        sender: &[u8],
        command: i32,
        raw_parameters: &[u8],
    ) -> IrcReduceResult {
        let mut result = IrcReduceResult::default();
        let mut parameters = Some(raw_parameters);
        let _target = extract_parameter(&mut parameters);
        let mut show_message = true;

        match command {
            433 => {
                let mut desired_nick = extract_parameter(&mut parameters);
                desired_nick.push(b'_');
                result
                    .outbound
                    .push(format_command(b"NICK", Some(&desired_nick)));
            }
            376 | 422 => {
                if let Some(auto_join) = &self.auto_join {
                    if !auto_join.is_empty() {
                        result
                            .outbound
                            .push(format_command(b"JOIN", Some(auto_join)));
                    }
                }
            }
            331 | 332 => {
                let channel = extract_parameter(&mut parameters);
                let topic = if command == 332 {
                    extract_parameter(&mut parameters)
                } else {
                    Vec::new()
                };
                let channel_index = self.add_channel(&channel);
                self.channels[channel_index].topic.clone_from(&topic);
                if !topic.is_empty() {
                    result.notifications += self.push_message(
                        IrcMessageType::Status,
                        sender,
                        &channel,
                        &join_bytes(&[b"Topic in ", &channel, b": ", &topic]),
                    );
                }
            }
            333 | 4 => show_message = false,
            353 => {
                let _symbol = extract_parameter(&mut parameters);
                let channel = extract_parameter(&mut parameters);
                let names = extract_parameter(&mut parameters);
                let channel_index = self.add_channel(&channel);
                self.channels[channel_index].receive_users(&names, &self.prefixes);
                show_message = false;
            }
            366 => {
                let channel = extract_parameter(&mut parameters);
                let channel_index = self.add_channel(&channel);
                self.channels[channel_index].receiving_users = false;
                show_message = false;
                result.notifications += 1;
            }
            5 => {
                while parameters.is_some_and(|remaining| !remaining.is_empty()) {
                    let token = extract_parameter(&mut parameters);
                    let (parameter, value) = split_once_byte(&token, b'=')
                        .map_or((token.as_slice(), &[][..]), |parts| parts);
                    if irc_eq(parameter, b"PREFIX") {
                        self.prefixes = value.to_vec();
                    }
                }
                show_message = false;
            }
            _ => {}
        }

        if show_message {
            let mut target_channel = None;
            if parameters
                .is_some_and(|remaining| !remaining.is_empty() && !remaining.starts_with(b":"))
            {
                let possible_channel = extract_parameter(&mut parameters);
                target_channel = self
                    .channel(&possible_channel)
                    .map(|channel| channel.name.clone());
            }

            let mut message = parameters;
            while message
                .is_some_and(|remaining| !remaining.is_empty() && !remaining.starts_with(b":"))
            {
                message = message.and_then(|remaining| {
                    remaining
                        .iter()
                        .position(|byte| *byte == b' ')
                        .map(|space| &remaining[space + 1..])
                });
            }
            if let Some(message) = message.and_then(|message| message.strip_prefix(b":")) {
                if let Some(channel) = target_channel {
                    result.notifications +=
                        self.push_message(IrcMessageType::Status, sender, &channel, message);
                } else {
                    let nick = self.nick.clone();
                    result.notifications +=
                        self.push_message(IrcMessageType::Server, sender, &nick, message);
                }
            }
        }

        result
    }

    fn receive_message(
        &mut self,
        notice: bool,
        sender: &[u8],
        target: &[u8],
        text: &[u8],
    ) -> IrcReduceResult {
        let mut result = IrcReduceResult::default();
        if let Some(mut remaining) = text.strip_prefix(b"\x01") {
            while !remaining.is_empty() {
                let (ctcp, next) = remaining
                    .iter()
                    .position(|byte| *byte == 1)
                    .map_or((remaining, &[][..]), |end| {
                        (&remaining[..end], &remaining[end + 1..])
                    });
                let (tag, data) =
                    split_once_byte(ctcp, b' ').map_or((ctcp, &[][..]), |parts| parts);
                let sender_nick = sender
                    .split(|byte| *byte == b'!')
                    .next()
                    .unwrap_or_default();
                if irc_eq(tag, b"ACTION") {
                    result.notifications +=
                        self.push_message(IrcMessageType::Action, sender, target, data);
                }
                if !notice && irc_eq(tag, b"VERSION") {
                    result.outbound.push(format_command(
                        b"NOTICE",
                        Some(&join_bytes(&[
                            sender_nick,
                            b" :\x01VERSION ",
                            &self.ctcp_version,
                            b"\x01",
                        ])),
                    ));
                }
                if !notice && irc_eq(tag, b"PING") {
                    result.outbound.push(format_command(
                        b"NOTICE",
                        Some(&join_bytes(&[sender_nick, b" :\x01PING ", data, b"\x01"])),
                    ));
                }
                remaining = next;
            }
        } else {
            result.notifications += self.push_message(
                if notice {
                    IrcMessageType::Notice
                } else {
                    IrcMessageType::Message
                },
                sender,
                target,
                text,
            );
        }
        result
    }

    fn add_channel(&mut self, name: &[u8]) -> usize {
        if let Some(index) = self
            .channels
            .iter()
            .position(|channel| irc_eq(&channel.name, name))
        {
            return index;
        }
        self.channels.insert(0, IrcChannel::new(name.to_vec()));
        0
    }

    fn remove_channel(&mut self, name: &[u8]) -> bool {
        let Some(index) = self
            .channels
            .iter()
            .position(|channel| irc_eq(&channel.name, name))
        else {
            return false;
        };
        self.channels.remove(index);
        true
    }

    fn pop_message(&mut self) {
        if self.messages.pop_front().is_none() {
            return;
        }
        self.unread_index = self.unread_index.saturating_sub(1);
    }

    fn push_message(
        &mut self,
        message_type: IrcMessageType,
        source: &[u8],
        target: &[u8],
        data: &[u8],
    ) -> usize {
        self.messages.push_back(IrcMessage {
            timestamp: SystemTime::now(),
            message_type,
            source: source.to_vec(),
            target: target.to_vec(),
            data: data.to_vec(),
        });
        while self.messages.len() > self.max_log_length {
            self.pop_message();
        }
        1
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum IrcClientError {
    #[error("not connected")]
    NotConnected,
    #[error("could not resolve IRC server: {0}")]
    Resolve(String),
    #[error("could not connect to IRC server: {0}")]
    Connect(String),
    #[error("IRC transport error: {0}")]
    Transport(String),
    #[error("IRC worker stopped")]
    WorkerStopped,
    #[error("invalid IRC state: {0}")]
    InvalidState(String),
}

#[derive(Debug)]
struct ParsedLine {
    prefix: Vec<u8>,
    command: Vec<u8>,
    parameters: Vec<u8>,
}

impl ParsedLine {
    fn parse(line: &[u8]) -> Option<Self> {
        let line = line.split(|byte| *byte == 0).next().unwrap_or_default();
        let (prefix, message) = if let Some(prefixed) = line.strip_prefix(b":") {
            let separator = prefixed.iter().position(|byte| *byte == b' ')?;
            (prefixed[..separator].to_vec(), &prefixed[separator + 1..])
        } else {
            (Vec::new(), line)
        };
        let message = &message[message
            .iter()
            .position(|byte| *byte != b' ')
            .unwrap_or(message.len())..];
        if message.is_empty() {
            return None;
        }
        let (command, parameters) = match message.iter().position(|byte| *byte == b' ') {
            Some(separator) => (
                message[..separator].to_vec(),
                message[separator + 1..].to_vec(),
            ),
            None => (message.to_vec(), Vec::new()),
        };
        Some(Self {
            prefix,
            command,
            parameters,
        })
    }
}

fn extract_parameter<'a>(parameters: &mut Option<&'a [u8]>) -> Vec<u8> {
    let Some(remaining) = *parameters else {
        return Vec::new();
    };
    if remaining.is_empty() {
        return Vec::new();
    }
    if let Some(trailing) = remaining.strip_prefix(b":") {
        *parameters = None;
        return trailing.to_vec();
    }
    if let Some(separator) = remaining.iter().position(|byte| *byte == b' ') {
        let result = remaining[..separator].to_vec();
        *parameters = Some(&remaining[separator + 1..]);
        result
    } else {
        *parameters = None;
        remaining.to_vec()
    }
}

fn format_command(command: &[u8], parameters: Option<&[u8]>) -> Vec<u8> {
    let command = c_string_bytes(command);
    parameters.map_or_else(
        || command.to_vec(),
        |parameters| join_bytes(&[command, b" ", c_string_bytes(parameters)]),
    )
}

fn irc_eq(left: &[u8], right: &[u8]) -> bool {
    let left = c_string_bytes(left);
    let right = c_string_bytes(right);
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(&left, &right)| irc_capital(left) == irc_capital(right))
}

fn irc_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - (b'a' - b'A'),
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

fn c_string_bytes(bytes: &[u8]) -> &[u8] {
    bytes.split(|byte| *byte == 0).next().unwrap_or_default()
}

fn truncate_at_nul(bytes: &mut Vec<u8>) {
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
}

fn join_bytes(parts: &[&[u8]]) -> Vec<u8> {
    let length = parts.iter().map(|part| part.len()).sum();
    let mut joined = Vec::with_capacity(length);
    for part in parts {
        joined.extend_from_slice(part);
    }
    joined
}

fn split_once_byte(bytes: &[u8], delimiter: u8) -> Option<(&[u8], &[u8])> {
    let separator = bytes.iter().position(|byte| *byte == delimiter)?;
    Some((&bytes[..separator], &bytes[separator + 1..]))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IrcLineDecoder {
    buffered: Vec<u8>,
}

impl IrcLineDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buffered.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(newline) = self.buffered.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffered.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            lines.push(line);
        }
        lines
    }

    pub fn buffered_len(&self) -> usize {
        self.buffered.len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IrcClientEvent {
    Connected,
    Notification,
    Disconnected { reason: String },
    Closed,
}

enum IrcWorkerRequest {
    Send {
        command: IrcCommand,
        completion: Sender<Result<(), IrcClientError>>,
    },
    Close {
        completion: Sender<Result<(), IrcClientError>>,
    },
}

/// A process-local IRC client backed by one blocking socket thread.
///
/// State snapshots are safe to read from the UI thread. Commands are written
/// by the worker before their local echo is committed to the reducer.
pub struct IrcClientHandle {
    state: Arc<Mutex<IrcClientState>>,
    commands: Sender<IrcWorkerRequest>,
    events: Receiver<IrcClientEvent>,
    cancelled: Arc<AtomicBool>,
    registration_complete: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl IrcClientHandle {
    pub fn connect(config: IrcConnectConfig) -> Result<Self, IrcClientError> {
        Self::connect_with_timeout(config, IRC_CONNECT_TIMEOUT)
    }

    pub fn connect_with_timeout(
        config: IrcConnectConfig,
        timeout: Duration,
    ) -> Result<Self, IrcClientError> {
        Self::connect_with_timeout_and_resolver(config, timeout, resolve_irc_servers)
    }

    fn connect_with_timeout_and_resolver<R>(
        config: IrcConnectConfig,
        timeout: Duration,
        resolver: R,
    ) -> Result<Self, IrcClientError>
    where
        R: FnOnce(&str) -> Result<Vec<SocketAddr>, IrcClientError> + Send + 'static,
    {
        let server = config.server.clone();
        let mut initial_state = IrcClientState::new();
        initial_state.begin_connect(config);
        let state = Arc::new(Mutex::new(initial_state));
        let cancelled = Arc::new(AtomicBool::new(false));
        let registration_complete = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker_state = Arc::clone(&state);
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_registration_complete = Arc::clone(&registration_complete);
        let worker = thread::Builder::new()
            .name("lc-irc".to_owned())
            .spawn(move || {
                run_irc_worker(
                    worker_state,
                    server,
                    timeout,
                    command_rx,
                    event_tx,
                    worker_cancelled,
                    worker_registration_complete,
                    resolver,
                );
            })
            .map_err(|error| IrcClientError::Transport(error.to_string()))?;
        Ok(Self {
            state,
            commands: command_tx,
            events: event_rx,
            cancelled,
            registration_complete,
            worker: Some(worker),
        })
    }

    pub fn snapshot(&self) -> IrcClientSnapshot {
        lock_state(&self.state).snapshot()
    }

    /// Copies the complete visible log and advances its read boundary while
    /// holding the reducer lock once. This prevents a worker message from
    /// being marked read between a UI snapshot and its acknowledgement.
    pub fn snapshot_and_mark_message_log_read(&self) -> IrcClientSnapshot {
        lock_state(&self.state).snapshot_and_mark_message_log_read()
    }

    pub fn is_active(&self) -> bool {
        lock_state(&self.state).is_active()
    }

    pub fn is_connected(&self) -> bool {
        lock_state(&self.state).is_connected()
    }

    pub fn mark_message_log_read(&self) {
        lock_state(&self.state).mark_message_log_read();
    }

    pub fn clear_message_log(&self) {
        lock_state(&self.state).clear_message_log();
    }

    pub fn events(&self) -> &Receiver<IrcClientEvent> {
        &self.events
    }

    pub fn try_recv_event(&self) -> Result<IrcClientEvent, TryRecvError> {
        self.events.try_recv()
    }

    pub fn recv_event_timeout(
        &self,
        timeout: Duration,
    ) -> Result<IrcClientEvent, RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    /// Queue a command without waiting for its socket write.
    pub fn queue_command(&self, command: IrcCommand) -> Result<(), IrcClientError> {
        if !self.is_connected() {
            return Err(IrcClientError::NotConnected);
        }
        let (completion, _result) = mpsc::channel();
        self.commands
            .send(IrcWorkerRequest::Send {
                command,
                completion,
            })
            .map_err(|_| IrcClientError::WorkerStopped)
    }

    /// Write a command and wait until the worker has committed its local echo.
    pub fn send_command(&self, command: IrcCommand) -> Result<(), IrcClientError> {
        if !self.is_connected() {
            return Err(IrcClientError::NotConnected);
        }
        let (completion, result) = mpsc::channel();
        self.commands
            .send(IrcWorkerRequest::Send {
                command,
                completion,
            })
            .map_err(|_| IrcClientError::WorkerStopped)?;
        result.recv().map_err(|_| IrcClientError::WorkerStopped)?
    }

    pub fn close(&mut self) -> Result<(), IrcClientError> {
        self.cancelled.store(true, Ordering::Release);
        let registration_complete = self.registration_complete.load(Ordering::Acquire);
        lock_state(&self.state).close();
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let (completion, result) = mpsc::channel();
        let request_result = self.commands.send(IrcWorkerRequest::Close { completion });
        if !registration_complete {
            // Name resolution and connect_timeout cannot be interrupted through
            // std. Detach the worker so closing the dialog never waits for
            // either; the cancellation flag prevents it from registering if
            // the pending operation later succeeds.
            return Ok(());
        }
        let close_result = if request_result.is_ok() {
            // The socket worker may have observed a remote close immediately
            // after accepting this request. Closing an already-stopped IRC
            // client is still successful, matching C4Network2IRCClient::Close.
            result.recv().unwrap_or(Ok(()))
        } else {
            Ok(())
        };
        let join_result = worker.join().map_err(|_| IrcClientError::WorkerStopped);
        close_result.and(join_result)
    }
}

impl Drop for IrcClientHandle {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

pub fn resolve_irc_server(server: &str) -> Result<SocketAddr, IrcClientError> {
    resolve_irc_servers(server)?
        .into_iter()
        .next()
        .ok_or_else(|| IrcClientError::Resolve(server.to_owned()))
}

fn resolve_irc_servers(server: &str) -> Result<Vec<SocketAddr>, IrcClientError> {
    let explicit_port = server.parse::<SocketAddr>().is_ok()
        || server
            .strip_prefix('[')
            .and_then(|server| server.split_once(']'))
            .is_some_and(|(_, suffix)| suffix.starts_with(':'))
        || (server.matches(':').count() == 1
            && server
                .rsplit_once(':')
                .is_some_and(|(_, port)| port.parse::<u16>().is_ok()));

    let addresses = if explicit_port {
        server.to_socket_addrs()
    } else {
        (server, IRC_DEFAULT_PORT).to_socket_addrs()
    }
    .map_err(|error| IrcClientError::Resolve(error.to_string()))?;
    let addresses = addresses.collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(IrcClientError::Resolve(server.to_owned()));
    }
    Ok(addresses)
}

fn connect_irc_addresses(
    addresses: &[SocketAddr],
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Option<TcpStream>, IrcClientError> {
    let mut last_error = None;
    for (index, address) in addresses.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return Ok(None);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let attempts_left = u32::try_from(addresses.len() - index).unwrap_or(u32::MAX);
        let mut attempt_timeout = remaining / attempts_left;
        if attempt_timeout.is_zero() {
            attempt_timeout = remaining;
        }
        match TcpStream::connect_timeout(address, attempt_timeout) {
            Ok(stream) => return Ok(Some(stream)),
            Err(error) => last_error = Some((*address, error)),
        }
    }
    if cancelled.load(Ordering::Acquire) {
        return Ok(None);
    }
    let detail = last_error.map_or_else(
        || "connection timed out".to_owned(),
        |(address, error)| format!("{address}: {error}"),
    );
    Err(IrcClientError::Connect(detail))
}

fn run_irc_worker<R>(
    state: Arc<Mutex<IrcClientState>>,
    server: String,
    connect_timeout: Duration,
    commands: Receiver<IrcWorkerRequest>,
    events: Sender<IrcClientEvent>,
    cancelled: Arc<AtomicBool>,
    registration_complete: Arc<AtomicBool>,
    resolver: R,
) where
    R: FnOnce(&str) -> Result<Vec<SocketAddr>, IrcClientError>,
{
    let started = Instant::now();
    if finish_cancelled_worker(&cancelled, &state, &events, None) {
        return;
    }
    let Some(deadline) = started.checked_add(connect_timeout) else {
        disconnect_worker(
            &state,
            &events,
            "IRC connect timeout exceeds the platform clock range".to_owned(),
        );
        return;
    };
    let addresses = match resolver(&server) {
        Ok(addresses) => addresses,
        Err(error) => {
            if !finish_cancelled_worker(&cancelled, &state, &events, None) {
                disconnect_worker(&state, &events, error.to_string());
            }
            return;
        }
    };
    if finish_cancelled_worker(&cancelled, &state, &events, None) {
        return;
    }
    let mut stream = match connect_irc_addresses(&addresses, deadline, &cancelled) {
        Ok(Some(stream)) => stream,
        Ok(None) => {
            finish_cancelled_worker(&cancelled, &state, &events, None);
            return;
        }
        Err(error) => {
            if !finish_cancelled_worker(&cancelled, &state, &events, None) {
                disconnect_worker(&state, &events, error.to_string());
            }
            return;
        }
    };
    if finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
        return;
    }
    if let Err(error) = configure_irc_stream(&stream) {
        if !finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
            disconnect_worker(&state, &events, error.to_string());
        }
        return;
    }
    if finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
        return;
    }

    let registration = match lock_state(&state).on_tcp_connected() {
        Ok(registration) => registration,
        Err(error) => {
            if !finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
                disconnect_worker(&state, &events, error.to_string());
            }
            return;
        }
    };
    if finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
        return;
    }
    for line in registration.outbound {
        if finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
            return;
        }
        if let Err(error) = write_irc_line(&mut stream, &line) {
            if !finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
                disconnect_worker(&state, &events, error.to_string());
            }
            return;
        }
    }
    registration_complete.store(true, Ordering::Release);
    let _ = events.send(IrcClientEvent::Connected);

    let mut decoder = IrcLineDecoder::default();
    let mut buffer = [0_u8; 8_192];
    loop {
        if finish_cancelled_worker(&cancelled, &state, &events, Some(&stream)) {
            return;
        }
        loop {
            match commands.try_recv() {
                Ok(request) => {
                    if process_worker_request(request, &state, &events, &mut stream) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = stream.shutdown(Shutdown::Both);
                    lock_state(&state).close();
                    let _ = events.send(IrcClientEvent::Closed);
                    return;
                }
            }
        }

        match stream.read(&mut buffer) {
            Ok(0) => {
                disconnect_worker(&state, &events, "connection closed".to_owned());
                return;
            }
            Ok(read) => {
                for line in decoder.push(&buffer[..read]) {
                    let reduced = lock_state(&state).receive_line(&line);
                    for outbound in reduced.outbound {
                        if let Err(error) = write_irc_line(&mut stream, &outbound) {
                            disconnect_worker(&state, &events, error.to_string());
                            return;
                        }
                    }
                    notify_updated(&events, reduced.notifications);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                disconnect_worker(&state, &events, error.to_string());
                return;
            }
        }
    }
}

fn configure_irc_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(IRC_READ_POLL_INTERVAL))?;
    stream.set_write_timeout(Some(IRC_WRITE_TIMEOUT))?;
    Ok(())
}

fn finish_cancelled_worker(
    cancelled: &AtomicBool,
    state: &Arc<Mutex<IrcClientState>>,
    events: &Sender<IrcClientEvent>,
    stream: Option<&TcpStream>,
) -> bool {
    if !cancelled.load(Ordering::Acquire) {
        return false;
    }
    if let Some(stream) = stream {
        let _ = stream.shutdown(Shutdown::Both);
    }
    lock_state(state).close();
    let _ = events.send(IrcClientEvent::Closed);
    true
}

fn process_worker_request(
    request: IrcWorkerRequest,
    state: &Arc<Mutex<IrcClientState>>,
    events: &Sender<IrcClientEvent>,
    stream: &mut TcpStream,
) -> bool {
    match request {
        IrcWorkerRequest::Send {
            command,
            completion,
        } => {
            let line = match lock_state(state).outgoing_line(&command) {
                Ok(line) => line,
                Err(error) => {
                    let _ = completion.send(Err(error));
                    return false;
                }
            };
            if let Err(error) = write_irc_line(stream, &line) {
                let error = IrcClientError::Transport(error.to_string());
                let _ = completion.send(Err(error.clone()));
                disconnect_worker(state, events, error.to_string());
                return true;
            }
            let reduced = lock_state(state).mark_outgoing_sent(&command);
            notify_updated(events, reduced.notifications);
            let quitting = matches!(command, IrcCommand::Quit { .. });
            let _ = completion.send(Ok(()));
            if quitting {
                let _ = stream.shutdown(Shutdown::Both);
                let _ = events.send(IrcClientEvent::Closed);
            }
            quitting
        }
        IrcWorkerRequest::Close { completion } => {
            let _ = stream.shutdown(Shutdown::Both);
            lock_state(state).close();
            let _ = completion.send(Ok(()));
            let _ = events.send(IrcClientEvent::Closed);
            true
        }
    }
}

fn write_irc_line(stream: &mut TcpStream, line: &[u8]) -> io::Result<()> {
    stream.write_all(line)?;
    stream.write_all(b"\r\n")
}

fn disconnect_worker(
    state: &Arc<Mutex<IrcClientState>>,
    events: &Sender<IrcClientEvent>,
    reason: String,
) {
    let reduced = lock_state(state).on_disconnected(reason.clone());
    notify_updated(events, reduced.notifications);
    let _ = events.send(IrcClientEvent::Disconnected { reason });
}

fn notify_updated(events: &Sender<IrcClientEvent>, notifications: usize) {
    for _ in 0..notifications {
        let _ = events.send(IrcClientEvent::Notification);
    }
}

fn lock_state(state: &Arc<Mutex<IrcClientState>>) -> MutexGuard<'_, IrcClientState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    fn test_config() -> IrcConnectConfig {
        IrcConnectConfig {
            server: "127.0.0.1".to_owned(),
            nick: b"Me".to_vec(),
            real_name: b"Clonk Player".to_vec(),
            password: None,
            auto_join: Some(b"#clonken,#legacyclonk".to_vec()),
            ctcp_version: b"Clonk Rust:test:unit".to_vec(),
        }
    }

    fn connected_state() -> IrcClientState {
        let mut state = IrcClientState::new();
        state.begin_connect(test_config());
        state.on_tcp_connected().expect("connect reducer");
        state
    }

    fn message_tuples(state: &IrcClientState) -> Vec<(IrcMessageType, Vec<u8>, Vec<u8>, Vec<u8>)> {
        state
            .messages()
            .map(|message| {
                (
                    message.message_type,
                    message.source.clone(),
                    message.target.clone(),
                    message.data.clone(),
                )
            })
            .collect()
    }

    fn read_wire_line(reader: &mut BufReader<TcpStream>) -> Vec<u8> {
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).expect("read IRC line");
        assert!(line.ends_with(b"\r\n"), "wire line lacked CRLF: {line:?}");
        line.truncate(line.len() - 2);
        line
    }

    fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(predicate(), "condition was not met before {timeout:?}");
    }

    #[test]
    fn line_decoder_preserves_fragmented_lf_and_crlf_frames() {
        let mut decoder = IrcLineDecoder::default();
        assert!(decoder.push(b":server PI").is_empty());
        assert_eq!(decoder.buffered_len(), 10);
        assert_eq!(
            decoder.push(b"NG :one\r\nNOTICE Me :two\npartial"),
            [b":server PING :one".to_vec(), b"NOTICE Me :two".to_vec()]
        );
        assert_eq!(decoder.buffered_len(), 7);
        assert_eq!(decoder.push(b"\r\n"), [b"partial".to_vec()]);
        assert_eq!(decoder.buffered_len(), 0);
    }

    #[test]
    fn registration_order_password_truncation_and_parameter_parsing_match_cpp() {
        assert_eq!(
            IrcConnectConfig::new("server", b"Clonker".to_vec(), b"Real Name".to_vec())
                .ctcp_version,
            format!("Clonk Rust:{IRC_CTCP_ENGINE_VERSION}:{}", c4_os_tag()).into_bytes()
        );
        let mut config = test_config();
        config.password = Some(b"1234567890123456789012345678901234567890".to_vec());
        let mut state = IrcClientState::new();
        state.begin_connect(config);
        assert_eq!(state.connection_state(), IrcConnectionState::Connecting);
        let connected = state.on_tcp_connected().expect("connected transition");
        assert_eq!(
            connected.outbound,
            [
                b"PASS 1234567890123456789012345678901".to_vec(),
                b"NICK Me".to_vec(),
                b"USER clonk x x :Clonk Player".to_vec(),
            ]
        );

        let result = state.receive_line(":Nick!ident PRIVMSG Me :hello there");
        assert_eq!(result.notifications, 1);
        assert_eq!(
            message_tuples(&state).last(),
            Some(&(
                IrcMessageType::Message,
                b"Nick!ident".to_vec(),
                b"Me".to_vec(),
                b"hello there".to_vec(),
            ))
        );

        // Only one delimiter space is skipped by ircExtractPar.
        state.receive_line(":Nick!ident PRIVMSG  Me :spaced");
        let last = state.messages().last().expect("spaced message");
        assert_eq!(last.target, b"");
        assert_eq!(last.data, b"Me");

        let mut nul_config = test_config();
        nul_config.nick = b"Me\0ignored".to_vec();
        nul_config.real_name = b"Real\0ignored".to_vec();
        nul_config.password = Some(b"secret\0ignored".to_vec());
        let mut nul_state = IrcClientState::new();
        nul_state.begin_connect(nul_config);
        assert_eq!(nul_state.user_name(), b"Me");
        assert_eq!(
            nul_state
                .on_tcp_connected()
                .expect("connect NUL-terminated native strings")
                .outbound,
            [
                b"PASS secret".to_vec(),
                b"NICK Me".to_vec(),
                b"USER clonk x x :Real".to_vec(),
            ]
        );
        assert!(irc_eq(b"#r\xe4um", b"#R\xc4UM"));
    }

    #[test]
    fn welcome_nick_collision_and_ping_emit_exact_commands() {
        let mut state = connected_state();
        assert_eq!(
            state.receive_line(":server 376 Me :End of MOTD").outbound,
            [b"JOIN #clonken,#legacyclonk".to_vec()]
        );
        assert_eq!(
            state.receive_line(":server 422 Me :MOTD missing").outbound,
            [b"JOIN #clonken,#legacyclonk".to_vec()]
        );
        assert_eq!(
            state
                .receive_line(":server 433 Me Desired :Nickname in use")
                .outbound,
            [b"NICK Desired_".to_vec()]
        );
        assert_eq!(state.user_name(), b"Me");
        assert_eq!(
            state.receive_line("PING :token value").outbound,
            [b"PONG :token value".to_vec()]
        );
    }

    #[test]
    fn prefix_capability_and_multibatch_names_lock_until_end() {
        let mut state = connected_state();
        let support =
            state.receive_line(":server 005 Me PREFIX=(qaohv)~&@%+ CHANTYPES=#+ :are supported");
        assert_eq!(support, IrcReduceResult::default());
        assert_eq!(state.prefixes(), b"(qaohv)~&@%+");

        let first = state.receive_line(":server 353 Me = #Room :@Alice +Bob");
        assert_eq!(first, IrcReduceResult::default());
        let room = state.channel("#room").expect("room from first NAMES");
        assert!(room.receiving_users);
        assert_eq!(
            room.users,
            [
                IrcUser {
                    prefix: b"+".to_vec(),
                    name: b"Bob".to_vec(),
                },
                IrcUser {
                    prefix: b"@".to_vec(),
                    name: b"Alice".to_vec(),
                },
            ]
        );

        state.receive_line(":server 353 Me = #room :~&Carol @+Dave");
        let room = state.channel("#ROOM").expect("same case-folded room");
        assert_eq!(room.user("Carol").expect("Carol").prefix, b"~&");
        assert_eq!(room.user("Dave").expect("Dave").prefix, b"@+");

        let end = state.receive_line(":server 366 Me #Room :End of NAMES");
        assert_eq!(end.notifications, 1);
        assert!(!state.channel("#room").expect("room").receiving_users);

        state.receive_line(":server 353 Me = #Room :%Erin");
        assert_eq!(
            state
                .channel("#room")
                .expect("room")
                .users
                .iter()
                .map(|user| user.name.as_slice())
                .collect::<Vec<_>>(),
            [b"Erin".as_slice()]
        );
        // This triggered undefined memory walking in C++; the Rust port ignores it.
        state.receive_line(":server 353 Me = #Room :@@");
        assert!(state.channel("#room").expect("room").user("").is_none());
    }

    #[test]
    fn join_part_kick_quit_and_nick_update_channel_state_on_server_echo() {
        let mut state = connected_state();
        state.receive_line(":Me!self JOIN :#One");
        state.receive_line(":Me!self JOIN :#Two");
        state.receive_line(":Peer!ident JOIN :#One");
        state.receive_line(":Peer!ident JOIN :#Two");
        assert!(state.channel("#one").expect("one").user("Peer").is_some());

        state.receive_line(":Peer!ident NICK :Renamed");
        for channel in ["#one", "#two"] {
            let channel = state.channel(channel).expect("renamed channel");
            assert!(channel.user("Peer").is_none());
            assert_eq!(channel.user("Renamed").expect("renamed user").prefix, b"");
        }
        state.receive_line(":Renamed!ident QUIT :gone");
        for channel in ["#one", "#two"] {
            assert!(state
                .channel(channel)
                .expect("quit channel")
                .user("Renamed")
                .is_none());
        }

        state.receive_line(":Me!self NICK :Myself");
        assert_eq!(state.user_name(), b"Myself");
        state.receive_line(":Myself!self PART #One :bye");
        assert!(state.channel("#one").is_none());
        assert!(state.channel("#two").is_some());
        state.receive_line(":Oper!ident KICK #Two Myself :rules");
        assert!(state.channel("#two").is_none());
        assert_eq!(state.connection_state(), IrcConnectionState::Connected);
    }

    #[test]
    fn topic_and_mode_update_status_and_resynchronize_existing_names() {
        let mut state = connected_state();
        state.receive_line(":Me!self JOIN :#room");
        let topic = state.receive_line(":Alice!ident TOPIC #ROOM :A new topic");
        assert_eq!(topic.notifications, 1);
        assert_eq!(state.channel("#room").expect("room").topic, b"A new topic");

        let mode = state.receive_line(":Oper!ident MODE #room +ov Alice");
        assert_eq!(mode.outbound, [b"NAMES #room".to_vec()]);
        assert_eq!(mode.notifications, 1);
        let unknown = state.receive_line(":Oper!ident MODE #absent +o Alice");
        assert!(unknown.outbound.is_empty());
        assert_eq!(unknown.notifications, 1);
    }

    #[test]
    fn numeric_messages_route_to_server_or_known_channel_and_honor_ignores() {
        let mut state = connected_state();
        state.receive_line(":Me!self JOIN :#room");
        assert_eq!(
            state
                .receive_line(":server 401 Me Missing :No such nick")
                .notifications,
            1
        );
        let server_message = state.messages().last().expect("server numeric");
        assert_eq!(server_message.message_type, IrcMessageType::Server);
        assert_eq!(server_message.target, b"Me");
        assert_eq!(server_message.data, b"No such nick");

        state.receive_line(":server 404 Me #ROOM :Cannot send");
        let channel_message = state.messages().last().expect("channel numeric");
        assert_eq!(channel_message.message_type, IrcMessageType::Status);
        assert_eq!(channel_message.target, b"#room");
        assert_eq!(channel_message.data, b"Cannot send");

        let before = state.messages().count();
        state.receive_line(":server 004 Me server version modes");
        state.receive_line(":server 333 Me #room setter 123");
        assert_eq!(state.messages().count(), before);

        state.receive_line(":server 331 Me #room :No topic is set");
        assert_eq!(state.channel("#room").expect("room").topic, b"");
        assert_eq!(
            state
                .messages()
                .last()
                .expect("331 generic text")
                .message_type,
            IrcMessageType::Server
        );
        state.receive_line(":server 332 Me #room :Topic from numeric");
        assert_eq!(
            state.channel("#room").expect("room").topic,
            b"Topic from numeric"
        );
    }

    #[test]
    fn privmsg_notice_and_ctcp_actions_and_replies_match_cpp() {
        let mut state = connected_state();
        state.receive_line(":Alice!ident PRIVMSG Me :hello");
        state.receive_line(":Alice!ident NOTICE Me :notice");
        let action = state.receive_line(":Alice!ident PRIVMSG #room :\u{1}ACTION waves\u{1}");
        assert_eq!(action.notifications, 1);
        assert!(action.outbound.is_empty());

        assert_eq!(
            state
                .receive_line(":Alice!ident PRIVMSG Me :\u{1}VERSION\u{1}")
                .outbound,
            [b"NOTICE Alice :\x01VERSION Clonk Rust:test:unit\x01".to_vec()]
        );
        assert_eq!(
            state
                .receive_line(":Alice!ident PRIVMSG Me :\u{1}PING 123 456\u{1}")
                .outbound,
            [b"NOTICE Alice :\x01PING 123 456\x01".to_vec()]
        );
        assert!(state
            .receive_line(":Alice!ident NOTICE Me :\u{1}VERSION reply\u{1}")
            .outbound
            .is_empty());

        let before = state.messages().count();
        let malformed = state.receive_line(":Alice!ident PRIVMSG Me :\u{1}UNKNOWN data");
        assert_eq!(malformed, IrcReduceResult::default());
        assert_eq!(state.messages().count(), before);
        assert_eq!(
            state
                .messages()
                .map(|message| message.message_type)
                .collect::<Vec<_>>(),
            [
                IrcMessageType::Message,
                IrcMessageType::Notice,
                IrcMessageType::Action,
            ]
        );
    }

    #[test]
    fn outbound_payloads_echo_only_after_success_and_wait_for_server_state_echoes() {
        let mut state = connected_state();
        let message = IrcCommand::Message {
            target: b"#room".to_vec(),
            text: b"hello".to_vec(),
        };
        assert_eq!(
            state.outgoing_line(&message).expect("message line"),
            b"PRIVMSG #room :hello"
        );
        assert!(state.messages().next().is_none());
        assert_eq!(state.mark_outgoing_sent(&message).notifications, 1);

        let action = IrcCommand::Action {
            target: b"Alice".to_vec(),
            text: b"waves".to_vec(),
        };
        assert_eq!(
            state.outgoing_line(&action).expect("action line"),
            b"PRIVMSG Alice :\x01ACTION waves\x01"
        );
        state.mark_outgoing_sent(&action);

        let join = IrcCommand::Join {
            channel: b"#later".to_vec(),
        };
        assert_eq!(
            state.outgoing_line(&join).expect("join line"),
            b"JOIN #later"
        );
        state.mark_outgoing_sent(&join);
        assert!(state.channel("#later").is_none());

        state.close();
        assert_eq!(
            state.outgoing_line(&message),
            Err(IrcClientError::NotConnected)
        );
    }

    #[test]
    fn message_log_caps_unread_boundary_and_channel_classification() {
        let mut state = IrcClientState::with_log_limits(3, 2);
        for index in 0..3 {
            state.push_message(
                IrcMessageType::Message,
                b"A",
                b"#room",
                index.to_string().as_bytes(),
            );
        }
        assert_eq!(state.messages().count(), 3);
        assert_eq!(state.unread_messages().count(), 3);
        state.mark_message_log_read();
        assert_eq!(
            state
                .messages()
                .map(|message| message.data.as_slice())
                .collect::<Vec<_>>(),
            [b"1".as_slice(), b"2".as_slice()]
        );
        assert_eq!(state.unread_messages().count(), 0);

        state.push_message(IrcMessageType::Notice, b"A", b"+local", b"3");
        assert_eq!(state.unread_messages().count(), 1);
        state.push_message(IrcMessageType::Message, b"A", b"&not-classic", b"4");
        assert_eq!(state.messages().count(), 3);
        assert_eq!(state.unread_messages().count(), 2);
        let messages = state.messages().collect::<Vec<_>>();
        assert!(messages[0].is_channel());
        assert!(messages[1].is_channel());
        assert!(!messages[2].is_channel());
        state.clear_message_log();
        assert_eq!(state.messages().count(), 0);
    }

    #[test]
    fn snapshot_and_mark_read_returns_every_message_before_advancing_the_boundary() {
        let mut state = IrcClientState::with_log_limits(3, 2);
        for index in 0..3 {
            state.push_message(
                IrcMessageType::Message,
                b"A",
                b"#room",
                index.to_string().as_bytes(),
            );
        }

        let snapshot = state.snapshot_and_mark_message_log_read();
        assert_eq!(snapshot.unread_index, 0);
        assert_eq!(
            snapshot
                .messages
                .iter()
                .map(|message| message.data.as_slice())
                .collect::<Vec<_>>(),
            [b"0".as_slice(), b"1".as_slice(), b"2".as_slice()]
        );
        assert_eq!(state.unread_messages().count(), 0);
        assert_eq!(
            state
                .messages()
                .map(|message| message.data.as_slice())
                .collect::<Vec<_>>(),
            [b"1".as_slice(), b"2".as_slice()]
        );

        state.push_message(IrcMessageType::Message, b"A", b"#room", b"3");
        let snapshot = state.snapshot_and_mark_message_log_read();
        assert_eq!(snapshot.unread_index, 2);
        assert_eq!(snapshot.messages[2].data, b"3");
        assert_eq!(state.unread_messages().count(), 0);
    }

    #[test]
    fn address_connect_falls_back_after_the_first_resolved_address_fails() {
        let live_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind live address");
        let live_address = live_listener.local_addr().expect("live address");
        let closed_listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve closed address");
        let closed_address = closed_listener.local_addr().expect("closed address");
        assert_ne!(closed_address, live_address);
        drop(closed_listener);

        let cancelled = AtomicBool::new(false);
        let stream = connect_irc_addresses(
            &[closed_address, live_address],
            Instant::now() + Duration::from_secs(2),
            &cancelled,
        )
        .expect("fallback connect")
        .expect("connect was not cancelled");
        assert_eq!(stream.peer_addr().expect("connected peer"), live_address);
        let (_accepted, _) = live_listener.accept().expect("accept fallback connection");
    }

    #[test]
    fn close_during_resolution_returns_immediately_and_never_registers() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind IRC listener");
        let address = listener.local_addr().expect("IRC listener address");
        let (resolver_started_tx, resolver_started_rx) = mpsc::channel();
        let (release_resolver_tx, release_resolver_rx) = mpsc::channel();
        let resolver = move |_server: &str| {
            resolver_started_tx.send(()).expect("signal resolver entry");
            release_resolver_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("release blocked resolver");
            Ok(vec![address])
        };

        let mut config = test_config();
        config.server = "resolver.test".to_owned();
        let mut handle = IrcClientHandle::connect_with_timeout_and_resolver(
            config,
            Duration::from_secs(2),
            resolver,
        )
        .expect("spawn IRC resolver worker");
        resolver_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("resolver entered on worker thread");

        let close_started = Instant::now();
        handle.close().expect("cancel pending IRC connect");
        assert!(
            close_started.elapsed() < Duration::from_millis(500),
            "close waited for the blocked resolver"
        );
        assert_eq!(
            handle.snapshot().connection_state,
            IrcConnectionState::Disconnected
        );

        release_resolver_tx
            .send(())
            .expect("release resolver after close");
        assert_eq!(
            handle.recv_event_timeout(Duration::from_secs(1)),
            Ok(IrcClientEvent::Closed)
        );
        listener
            .set_nonblocking(true)
            .expect("make listener nonblocking");
        assert!(matches!(
            listener.accept(),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn loopback_worker_registers_replies_and_commits_local_and_remote_messages() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback IRC");
        let address = listener.local_addr().expect("loopback address");
        let (registered_tx, registered_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept IRC client");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("server read timeout");
            let mut reader = BufReader::new(stream.try_clone().expect("clone server stream"));
            assert_eq!(read_wire_line(&mut reader), b"NICK Me");
            assert_eq!(read_wire_line(&mut reader), b"USER clonk x x :Clonk Player");

            stream
                .write_all(b":server 376 Me :End of MOTD\r\nPING :probe\r\n")
                .expect("send welcome and ping");
            assert_eq!(read_wire_line(&mut reader), b"JOIN #loopback");
            assert_eq!(read_wire_line(&mut reader), b"PONG :probe");
            registered_tx.send(()).expect("registration signal");
            assert_eq!(
                read_wire_line(&mut reader),
                b"PRIVMSG #loopback :from client"
            );
            stream
                .write_all(b":Alice!ident PRIVMSG #loopback :from server\r\n")
                .expect("send remote message");
            finished_tx.send(()).expect("finished signal");
            thread::sleep(Duration::from_millis(50));
        });

        let mut config = test_config();
        config.server = address.to_string();
        config.auto_join = Some(b"#loopback".to_vec());
        let mut handle = IrcClientHandle::connect_with_timeout(config, Duration::from_secs(2))
            .expect("start IRC handle");
        assert_eq!(
            handle.recv_event_timeout(Duration::from_secs(2)),
            Ok(IrcClientEvent::Connected)
        );
        registered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("server observed registration");
        handle
            .send_command(IrcCommand::Message {
                target: b"#loopback".to_vec(),
                text: b"from client".to_vec(),
            })
            .expect("send local message");
        finished_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("server completed exchange");
        wait_until(Duration::from_secs(2), || {
            handle
                .snapshot()
                .messages
                .iter()
                .any(|message| message.data == b"from server")
        });
        let snapshot = handle.snapshot();
        assert!(snapshot.messages.iter().any(|message| {
            message.message_type == IrcMessageType::Message
                && message.source == b"Me"
                && message.data == b"from client"
        }));
        assert!(snapshot.messages.iter().any(|message| {
            message.message_type == IrcMessageType::Message
                && message.source == b"Alice!ident"
                && message.data == b"from server"
        }));
        server.join().expect("loopback server");
        handle.close().expect("close IRC handle");
    }

    #[test]
    fn irc_transport_preserves_legacy_bytes_and_password_cap() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind byte-exact IRC");
        let address = listener.local_addr().expect("byte-exact IRC address");
        let (outbound_seen_tx, outbound_seen_rx) = mpsc::channel();
        let (release_server_tx, release_server_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept byte-exact IRC client");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("byte-exact server read timeout");
            let mut reader = BufReader::new(stream.try_clone().expect("clone byte-exact stream"));

            let mut expected_password = b"PASS ".to_vec();
            expected_password.extend_from_slice(&[b'p'; 30]);
            // The 31-byte C++ cap intentionally leaves the leading byte of the
            // final UTF-8 sequence rather than backing up to a character boundary.
            expected_password.push(0xc3);
            assert_eq!(expected_password.len(), b"PASS ".len() + 31);
            assert!(std::str::from_utf8(&expected_password[b"PASS ".len()..]).is_err());
            assert_eq!(read_wire_line(&mut reader), expected_password);
            assert_eq!(read_wire_line(&mut reader), b"NICK M\xe4");
            assert_eq!(read_wire_line(&mut reader), b"USER clonk x x :R\xe9al");

            // Split a prefixed command across writes and then send channel,
            // query, topic, source, target, and payload bytes outside UTF-8.
            stream
                .write_all(b":M\xe4!self JOIN :#r\xe4um\r\n:Al\xed")
                .expect("send first legacy byte fragment");
            stream
                .write_all(&join_bytes(&[
                    b"ce!ident TOPIC #r\xc4um :t\xf6pic\r\n",
                    b":Al\xedce!ident PRIVMSG #r\xe4um :inbound \x80\r\n",
                    b":Al\xedce!ident PRIVMSG M\xe4 :query \x81\r\n",
                    b":Al\xedce!ident PRIVMSG M\xe4 :\x01VERSION\x01\r\n",
                ]))
                .expect("send remaining legacy byte frames");

            assert_eq!(
                read_wire_line(&mut reader),
                b"NOTICE Al\xedce :\x01VERSION Clonk Rust:legacy:\x80\x01"
            );
            assert_eq!(
                read_wire_line(&mut reader),
                b"PRIVMSG #r\xe4um :outbound \x96"
            );
            outbound_seen_tx
                .send(())
                .expect("signal byte-exact outbound frame");
            release_server_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("release byte-exact IRC server");
        });

        let mut password = vec![b'p'; 30];
        password.extend_from_slice(&[0xc3, 0xa9]);
        let mut config =
            IrcConnectConfig::new(address.to_string(), b"M\xe4".to_vec(), b"R\xe9al".to_vec());
        config.password = Some(password);
        config.ctcp_version = b"Clonk Rust:legacy:\x80".to_vec();
        let mut handle = IrcClientHandle::connect_with_timeout(config, Duration::from_secs(2))
            .expect("start byte-exact IRC handle");
        assert_eq!(
            handle.recv_event_timeout(Duration::from_secs(2)),
            Ok(IrcClientEvent::Connected)
        );

        wait_until(Duration::from_secs(2), || {
            let snapshot = handle.snapshot();
            snapshot
                .channels
                .iter()
                .any(|channel| channel.name == b"#r\xe4um" && channel.topic == b"t\xf6pic")
                && snapshot.messages.iter().any(|message| {
                    message.message_type == IrcMessageType::Message
                        && message.source == b"Al\xedce!ident"
                        && message.target == b"#r\xe4um"
                        && message.data == b"inbound \x80"
                })
                && snapshot.messages.iter().any(|message| {
                    message.message_type == IrcMessageType::Message
                        && message.source == b"Al\xedce!ident"
                        && message.target == b"M\xe4"
                        && message.data == b"query \x81"
                })
        });

        handle
            .send_command(IrcCommand::Message {
                target: b"#r\xe4um".to_vec(),
                text: b"outbound \x96".to_vec(),
            })
            .expect("send byte-exact local message");
        outbound_seen_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("server observed byte-exact local message");

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.nick, b"M\xe4");
        assert!(snapshot.messages.iter().any(|message| {
            message.message_type == IrcMessageType::Message
                && message.source == b"M\xe4"
                && message.target == b"#r\xe4um"
                && message.data == b"outbound \x96"
        }));

        release_server_tx
            .send(())
            .expect("release byte-exact IRC server");
        server.join().expect("byte-exact IRC server");
        handle.close().expect("close byte-exact IRC handle");
    }

    #[test]
    fn loopback_connect_failure_becomes_inactive_disconnected_status() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve loopback port");
        let address = listener.local_addr().expect("reserved address");
        drop(listener);

        let mut config = test_config();
        config.server = address.to_string();
        let mut handle = IrcClientHandle::connect_with_timeout(config, Duration::from_millis(250))
            .expect("spawn failing IRC handle");
        let event = loop {
            match handle.recv_event_timeout(Duration::from_secs(2)) {
                Ok(IrcClientEvent::Notification) => continue,
                other => break other,
            }
        };
        assert!(matches!(event, Ok(IrcClientEvent::Disconnected { .. })));
        let snapshot = handle.snapshot();
        assert_eq!(snapshot.connection_state, IrcConnectionState::Disconnected);
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].message_type, IrcMessageType::Status);
        assert!(snapshot.messages[0]
            .data
            .starts_with(b"Disconnected from server ("));
        handle.close().expect("close failed handle");
    }

    #[test]
    fn malformed_lines_and_prefix_only_names_are_safe_noops() {
        let mut state = connected_state();
        for line in [
            "",
            ":prefix-without-command",
            "   ",
            "NOTICE",
            "1A2 Me :atoi-compatible numeric",
            ":server 353 Me = #room :@@",
            ":server 005 Me PREFIX :supported",
            "\0PRIVMSG Me :hidden",
        ] {
            state.receive_line(line);
        }
        assert!(state
            .channel("#room")
            .expect("malformed NAMES still creates room")
            .users
            .is_empty());
    }

    #[test]
    fn resolver_applies_default_port_and_retains_explicit_port() {
        assert_eq!(
            resolve_irc_server("127.0.0.1").expect("default loopback resolution"),
            SocketAddr::from(([127, 0, 0, 1], IRC_DEFAULT_PORT))
        );
        assert_eq!(
            resolve_irc_server("127.0.0.1:16667").expect("explicit loopback resolution"),
            SocketAddr::from(([127, 0, 0, 1], 16667))
        );
    }
}
