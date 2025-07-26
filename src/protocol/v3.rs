pub const PROTOCOL_NAME: &str = "MQTT";
pub const PROTOCOL_LEVEL: u8 = 4; // MQTT v3.1.1

// Connection return codes for MQTT v3.1.1
pub mod connect_return_codes {
    pub const ACCEPTED: u8 = 0x00;
    pub const UNACCEPTABLE_PROTOCOL_VERSION: u8 = 0x01;
    pub const IDENTIFIER_REJECTED: u8 = 0x02;
    pub const SERVER_UNAVAILABLE: u8 = 0x03;
    pub const BAD_USERNAME_OR_PASSWORD: u8 = 0x04;
    pub const NOT_AUTHORIZED: u8 = 0x05;
}

// Subscribe return codes for MQTT v3.1.1
pub mod subscribe_return_codes {
    pub const MAXIMUM_QOS_0: u8 = 0x00;
    pub const MAXIMUM_QOS_1: u8 = 0x01;
    pub const MAXIMUM_QOS_2: u8 = 0x02;
    pub const FAILURE: u8 = 0x80;
}
