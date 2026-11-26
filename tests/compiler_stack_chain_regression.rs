//! Keep long bounded chains and branches executable; reject over-budget
//! source without aborting the compiler. No test stack-size increase.
use atto_compiler::compile;
#[test]
fn long_chain_on_small_host_stack() {
    let handle=std::thread::Builder::new().stack_size(2*1024*1024).spawn(|| {
        let chain=|n:usize|format!("pub fnc main(): int {{ return {}; }}",vec!["1";n].join(" + "));
        let guest=compile(&chain(250)).expect("250 terms must remain admitted");
        assert_eq!(&guest[..8],b"\0asm\x01\0\0\0");
        assert_eq!(compile(&chain(3000)).unwrap_err().code,"E_NESTING");
    }).unwrap();
    handle.join().unwrap();
}
#[test]
fn long_elif_on_small_host_stack() {
    let handle=std::thread::Builder::new().stack_size(2*1024*1024).spawn(|| {
        let make=|n:usize| {
            let arms=(1..n).map(|i|format!("elif x eq {i} then return {i};")).collect::<Vec<_>>().join("\n");
            format!("pub fnc main(): int {{\nval x = 249;\nif x eq 0 then return 0;\n{arms}\nfi\nreturn 0;\n}}")
        };
        let guest=compile(&make(250)).expect("250 branches must remain admitted");
        assert_eq!(&guest[..8],b"\0asm\x01\0\0\0");
        assert_eq!(compile(&make(300)).unwrap_err().code,"E_NESTING");
    }).unwrap();
    handle.join().unwrap();
}
