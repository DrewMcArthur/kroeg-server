extern crate dotenv;
extern crate futures_await as futures;
extern crate hyper;
extern crate kroeg;

use futures::{future, Future};
use hyper::{Body, Response, Server};
use kroeg::{config, context, get, post, router::Route, KroegServiceBuilder};

fn main() {
    dotenv::dotenv().ok();
    let config = config::read_config();

    let addr = &config.listen.parse().expect("Invalid listen address!");

    let routes = vec![
        Route::get_prefix("/", kroeg::compact_response(get::get)),
        Route::post_prefix("/", kroeg::compact_response(post::post)),
        Route::get(
            "/-/context",
            Box::new(|_, store, queue, _| {
                Box::new(future::ok((
                    store,
                    queue,
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/ld+json")
                        .body(Body::from(context::read_context().to_string()))
                        .unwrap(),
                )))
            }),
        ),
    ];

    let mut builder = KroegServiceBuilder {
        config: config.clone(),
        routes: routes,
    };

    kroeg::webfinger::register(&mut builder);

    let server = Server::bind(&addr).serve(builder);

    println!("Kroeg v{} starting...", env!("CARGO_PKG_VERSION"));
    println!("listening at {}", addr);

    hyper::rt::run(hyper::rt::lazy(move || {
        hyper::rt::spawn(server.map_err(|_| {}));
        hyper::rt::spawn(kroeg::launch_delivery(config.clone()));

        Ok(())
    }))
}
