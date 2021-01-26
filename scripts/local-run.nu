# This script can be used to set the appropriate environment 
# variables and run the program locally.

let-env INPUT_REQUIRED-STATUSES = hello
let-env GITHUB_REPOSITORY = gleam-lang/gleam
let-env INPUT_CI-PASSED-LABEL = ci-passed

watchexec cargo run
