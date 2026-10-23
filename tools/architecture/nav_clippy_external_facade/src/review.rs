//! Compiler-visible review fixtures. This module is not linted.
//! The consumer probes import its public aliases.
// Rustdoc JSON resolves the public alias forms in this fixture crate.
pub use client::io;

// probe: EXTERNAL_CLIENT_IO_ALIAS
pub mod external_client_io_alias {
    pub type Probe = client::io::ClientProt;
}

// probe: EXTERNAL_CLIENT_ROOT_ALIAS
pub mod external_client_root_alias {
    pub type Probe = client::CheatAdmission;
}

// probe: EXTERNAL_BARE_ALIAS
pub use nav_clippy_external_facade::Alias as BareAlias;

// probe: EXTERNAL_BOX_ALIAS_LIMIT
pub mod external_box_alias_limit {
    pub type Probe = nav_clippy_external_facade::BoxAlias;
}

// probe: R1-A
pub mod r1_a_root_glob {
    pub use client::io::*;
    pub type Probe = client_prot::ClientProt;
}

// probe: R1-B
pub mod r1_b_prelude_glob {
    pub use client::io;
    pub mod prelude {
        pub use crate::*;
    }
    pub type Probe = prelude::io::client_prot::ClientProt;
}

// probe: R1-C
pub mod r1_c_root_alias {
    pub use client::io as wire;
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: N1
pub mod n1_alias_of_alias {
    pub use client::io as wire;
    pub mod m {
        pub use super::wire as w2;
    }
    pub type Probe = m::w2::client_prot::ClientProt;
}

// probe: N2
pub mod n2_self_glob_in_reexported_module {
    pub mod outer {
        pub mod inner {
            pub mod client_prot {
                pub type ClientProt = client::io::client_prot::ClientProt;
            }
        }
        pub use self::inner::*;
    }
    pub use outer as facade;
    pub type Probe = facade::client_prot::ClientProt;
}

// probe: N3
pub mod n3_nested_public_modules {
    pub mod a {
        pub mod b {
            pub use client::io::*;
        }
    }
    pub use a as x;
    pub type Probe = x::b::client_prot::ClientProt;
}

// probe: N4
pub mod n4_glob_of_module_alias {
    pub use client::io as wire;
    pub mod p {
        pub use super::wire::*;
    }
    pub type Probe = p::client_prot::ClientProt;
}

// probe: N5
pub mod n5_group_self_rename {
    pub use client::io::{self as wire};
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: N6
pub mod n6_crate_root_alias {
    pub use crate as root;
    pub use client::io;
    pub type Probe = root::io::client_prot::ClientProt;
}

// probe: N7
pub mod n7_super_alias {
    pub use client::io;
    pub mod m {
        pub use super::io as wire;
    }
    pub type Probe = m::wire::client_prot::ClientProt;
}

// probe: N8
pub mod n8_cross_crate_alias {
    pub use client as c;
    pub type Probe = c::io::client_prot::ClientProt;
}

// probe: N9
pub mod n9_cross_crate_glob {
    pub use ::client::*;
    pub type Probe = io::client_prot::ClientProt;
}

// probe: N10
pub mod n10_glob_copies_alias {
    pub use client::io;
    pub mod a {
        pub use crate::io as wire;
    }
    pub use a::*;
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: N11
pub mod n11_pub_extern_crate_alias {
    pub extern crate client as c;
    pub type Probe = c::io::client_prot::ClientProt;
}

// probe: N12
pub mod n12_pub_extern_crate_self_alias {
    pub use client::io;
    pub extern crate self as me;
    pub type Probe = me::io::client_prot::ClientProt;
}

// probe: T0
pub mod t0_pub_use_type {
    pub use client::io::client_prot::ClientProt as Prot;
    pub type Probe = Prot;
}

// probe: T1
pub mod t1_pub_type_alias {
    pub type Prot = client::io::client_prot::ClientProt;
}

// probe: T2
pub mod t2_cross_crate_type_alias {
    pub type Prot = client::io::client_prot::ClientProt;
}

// probe: M1
pub mod m1_item_macro_reexport {
    macro_rules! reexport {
        () => {
            pub use client::io as wire;
        };
    }
    reexport!();
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: M2
pub mod m2_include_reexport {
    include!("m2.rs");
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: M3
pub use client as client_alias;
#[macro_export]
macro_rules! m3_exported_type {
    () => {
        $crate::client_alias::io::client_prot::ClientProt
    };
}
pub mod m3_exported_macro {
    pub type Probe = crate::m3_exported_type!();
}

// probe: E1
pub mod e1_nested_extern_crate {
    pub mod m {
        pub extern crate client;
        pub type Probe = client::io::client_prot::ClientProt;
    }
}

// probe: E3
pub mod e3_leading_absolute_alias {
    pub use ::client as c2;
    pub type Probe = c2::io::client_prot::ClientProt;
}

// probe: E4
pub mod e4_private_extern_crate_prefix {
    extern crate client as c;
    pub use c::io as wire;
}

// probe: G0
pub mod g0_plain_alias {
    pub type W = client::io::client_prot::ClientProt;
}

// probe: G1
pub mod g1_box_argument {
    pub type W = Box<client::io::client_prot::ClientProt>;
}

// probe: G2
pub mod g2_option_argument {
    pub type W = Option<client::io::client_prot::ClientProt>;
}

// probe: G3
pub mod g3_generic_default {
    pub type W<T = client::io::client_prot::ClientProt> = Vec<T>;
}

// probe: G4
pub mod g4_dyn_trait_argument {
    pub type W = Box<dyn api::prot::Out>;
}

// probe: G5
pub mod g5_reference_alias {
    pub type W = &'static client::io::client_prot::ClientProt;
}

// probe: G6
pub mod g6_private_module_reexport {
    mod m {
        pub type X = client::io::client_prot::ClientProt;
    }
    pub use m::X as Y;
}

// probe: G7
pub mod g7_alias_chain {
    pub type A = client::io::client_prot::ClientProt;
    pub type B = A;
}

// probe: G8
pub mod g8_associated_type_projection {
    pub struct X;
    pub trait T {
        type A;
    }
    impl T for X {
        type A = api::prot::Send;
    }
}

// probe: G9
pub mod g9_harmless_alias {
    pub type W = Vec<u8>;
}

// probe: G7a
pub mod g7a_bare_alias_chain {
    pub type A = client::io::client_prot::ClientProt;
    pub type B = A;
}

// probe: G7b
pub mod g7b_use_of_alias {
    pub type A = client::io::client_prot::ClientProt;
    pub use A as C;
}

// probe: G7c
pub mod g7c_self_alias_control {
    pub type A = client::io::client_prot::ClientProt;
    pub type B = self::A;
}

// probe: G7d
pub mod g7d_use_alias_control {
    pub use client::io::client_prot::ClientProt as A;
    pub use A as C;
}

// probe: S1
pub mod s1_shadowed_thread_local {
    macro_rules! thread_local {
        () => {
            pub use client::io as wire;
        };
    }
    thread_local!();
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: S2
pub mod s2_shadowed_include {
    macro_rules! include {
        ($file:literal) => {
            pub use client::io as wire;
        };
    }
    include!("not-read.rs");
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: S3
#[macro_export]
macro_rules! s3_reexport_macro {
    () => {
        pub use client::io as wire;
    };
}
pub mod s3_macro_alias {
    pub use crate::s3_reexport_macro as thread_local;
    thread_local!();
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: I1
pub mod i1_std_include {
    std::include!("i1.rs");
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: I2
pub mod i2_core_include {
    core::include!("i2.rs");
    pub type Probe = wire::client_prot::ClientProt;
}

// probe: I3
pub mod i3_nested_include {
    pub mod inner {
        include!("i3.rs");
    }
    pub type Probe = inner::Prot;
}

// probe: I4
pub mod i4_non_path_include {
    include!("i4.rs");
}

// probe: I5
pub mod i5_included_item_macro {
    macro_rules! other {
        () => {
            pub type Probe = client::io::client_prot::ClientProt;
        };
    }
    include!("i5.rs");
}

// probe: A1
pub mod a1_pub_module_alias {
    pub use client::io as wire;
    pub type W = Option<wire::client_prot::ClientProt>;
}

// probe: A2
pub mod a2_private_module_alias {
    use client::io as cio;
    pub type W = Box<cio::client_prot::ClientProt>;
}

// probe: A3
pub mod a3_private_alias_of_reexport {
    mod a {
        pub use client::io::client_prot;
    }
    use a::client_prot as cp;
    pub type W = Box<cp::ClientProt>;
}

// probe: A4
pub mod a4_restricted_module_import {
    mod a {
        pub(crate) use client::io::client_prot;
    }
    pub type W = a::client_prot::ClientProt;
}

// probe: A5
pub mod a5_restricted_import_public_use {
    mod a {
        pub(crate) use client::io::client_prot;
    }
    pub use a::client_prot::ClientProt as P;
}

// probe: A6
pub mod a6_parent_glob_type_alias {
    use client::io::*;
    pub type W = client_prot::ClientProt;
}

// probe: A7
pub mod a7_parent_glob_pub_use {
    use client::io::*;
    pub use client_prot::ClientProt as P;
}

// probe: A8
pub mod a8_bare_item_from_glob {
    use client::io::client_prot::*;
    pub type W = ClientProt;
}

// probe: A9
pub mod a9_self_item_from_glob {
    use client::io::client_prot::*;
    pub type W = self::ClientProt;
}

// probe: B4
pub mod b4_value_only_control {
    pub type W = [u8; crate::review_seed::io::client_prot::N];
}

// probe: C1
pub mod c1_module_alias_with_box {
    pub mod inner {
        pub type W = Box<client::io::client_prot::ClientProt>;
    }
    pub use inner as facade;
}

// probe: C2
pub mod c2_module_alias_chain {
    pub mod inner {
        pub type W = Option<client::io::client_prot::ClientProt>;
    }
    pub use facade::W as X;
    pub use inner as facade;
}

// probe: C3
pub mod c3_glob_generic_default {
    mod inner {
        pub type W<T = client::io::client_prot::ClientProt> = Vec<T>;
    }
    pub use inner::*;
}

// probe: C4
pub mod c4_facade_reexport {
    mod inner {
        pub type W = (client::io::client_prot::ClientProt, u8);
    }
    pub mod facade {
        pub use super::inner::W;
    }
}

// probe: D1
pub mod d1_private_alias_step {
    type A = client::io::client_prot::ClientProt;
    pub type B = Box<A>;
}

// probe: D2
pub mod d2_restricted_alias_step {
    pub(crate) type A = client::io::client_prot::ClientProt;
    pub type B = A;
}

// probe: D3
pub mod d3_nested_module_shadow {
    pub mod a {}
    pub mod outer {
        pub mod a {
            pub type X = client::io::client_prot::ClientProt;
        }
        pub type W = a::X;
    }
}

// probe: D4
pub mod d4_raw_crate_identifier {
    pub type W = r#client::io::client_prot::ClientProt;
}

// probe: D5
pub mod d5_raw_crate_reexport {
    pub use r#client::io as wire;
    pub type W = wire::client_prot::ClientProt;
}

// probe: D6
pub mod d6_raw_planner_path {
    pub type W = r#client::io::client_prot::ClientProt;
}

// probe: D7
pub mod d7_raw_macro_identifier {
    macro_rules! r#thread_local {
        () => {
            pub use client::io as wire;
        };
    }
    thread_local!();
    pub type W = wire::client_prot::ClientProt;
}

// probe: D8
pub mod d8_alias_where_clause {
    #[expect(
        type_alias_bounds,
        reason = "this probe reproduces the reviewed type-alias where clause"
    )]
    pub type W<T>
    where
        T: crate::review_seed::io::client_prot::Act,
    = Vec<T>;
}

// probe: D9
pub mod d9_extern_crate_rename {
    extern crate client as c;
    pub type W = Box<c::io::client_prot::ClientProt>;
}

// probe: D10
pub mod d10_dyn_associated_type {
    pub trait Tr {
        type A;
    }
    pub type W = Box<dyn Tr<A = client::io::client_prot::ClientProt>>;
}
