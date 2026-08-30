#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod api;
pub mod frb_generated;

pub use api::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_open_default() {
        let res = api::engine_open("".to_string());
        println!("engine_open result: {:?}", res);
        assert!(res.is_ok(), "engine_open failed: {:?}", res);
    }
}
