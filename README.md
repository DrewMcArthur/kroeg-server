# Kroeg

> kroeg *noun*
>  - pub
>  - bar

a generic ActivityPub server, with a focus (for now) on microblogging style activities.

# Setup
For now, I would of course only suggest you run this for development, and not trust arbitrary users with an account. There is also no login system.

anyways, if you still want to try it:
 - Install the latest Rust nightly, or close to it
 - Create a folder
 - clone https://github.com/kroeg/jsonld-rs, https://git.puckipedia.com/kroeg/tap, https://git.puckipedia.com/kroeg/cellar, and https://git.puckipedia.com/kroeg/server into this folder
 - Go to cellar, put the `.sql` file into your database.
 - Copy `server.toml.example` to `server.toml`, set it up as required.
 - Create root users with `cargo run --bin kroeg-call create https://example.com/~exampleUser exampleUser "Example User"`
 - Gain an authorization key by running `cargo run --bin kroeg-call auth https://example.com/~exampleUser`
 - Use this in the Authorization header in any requests
 - Run `cargo run --bin kroeg` to actually run the server.

