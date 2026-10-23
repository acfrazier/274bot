extern crate self as nav_clippy_external_facade;

pub use client as client_alias;
pub use client::io;

pub type Alias = client::io::client_prot::ClientProt;
pub type BoxAlias = Box<client::io::client_prot::ClientProt>;
pub type SendAlias = api::prot::Send;

pub mod values {
    pub mod static_value {
        pub static S: api::prot::Send = api::prot::Send {
            prot: client::io::client_prot::ClientProt::NO_TIMEOUT,
            value: 0,
        };
    }

    pub mod const_value {
        pub const C: api::prot::Send = api::prot::Send {
            prot: client::io::client_prot::ClientProt::NO_TIMEOUT,
            value: 0,
        };
    }

    pub mod associated_value {
        pub struct S;

        impl S {
            pub const C: api::prot::Send = api::prot::Send {
                prot: client::io::client_prot::ClientProt::NO_TIMEOUT,
                value: 0,
            };
        }
    }
}

pub struct Projection;

pub trait Project {
    type Item;
}

impl Project for Projection {
    type Item = api::prot::Send;
}

#[macro_export]
macro_rules! export_client_type {
    () => {
        pub type MacroType = client::io::client_prot::ClientProt;
    };
}

pub mod review;

pub mod review_seed {
    pub mod io {
        pub mod client_prot {
            pub struct ClientProt;
            pub trait Act {}
            pub const N: usize = 1;
        }
    }
}
