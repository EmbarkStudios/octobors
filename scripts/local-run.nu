# This script can be used to set the appropriate environment 
# variables and run the program locally.

let-env INPUT_REQUIRED_STATUSES = lint,test
let-env GITHUB_REPOSITORY = lpil/puter
let-env INPUT_CI_PASSED_LABEL = ci-passed
let-env INPUT_NEEDS_DESCRIPTION_LABEL = needs-description

watchexec cargo run
