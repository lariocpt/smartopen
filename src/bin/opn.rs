// `opn` is the short alias for `smartopen`: the same program under the three-character
// name that existing yazi, broot and niri configurations invoke.
fn main() {
    std::process::exit(smartopen::main_exit_code());
}
