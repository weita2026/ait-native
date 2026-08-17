use std::net::SocketAddr;
use std::process;

use ait_server::installed_lifecycle::{
    lifecycle_usage, parse_lifecycle_args, prepare_installed_lifecycle, LifecycleCommand,
    PreparedLifecycle,
};
use ait_server::{
    build_router, build_startup_router, ensure_durable_runtime_access,
    ensure_startup_runtime_access, initialize_installed_runtime,
};
use ait_server_core::foundation::server_binary_lifecycle::ServerBinaryLifecycleConfig;
use axum::Server;

#[tokio::main]
async fn main() {
    let options = parse_lifecycle_args(std::env::args_os().skip(1)).unwrap_or_else(|message| {
        eprintln!("ait-server: {message}\n\n{}", lifecycle_usage());
        process::exit(64);
    });
    if options.command == LifecycleCommand::Version {
        println!("ait-server {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if options.command == LifecycleCommand::Help {
        println!("{}", lifecycle_usage());
        return;
    }
    let prepared = prepare_installed_lifecycle(options).unwrap_or_else(|message| {
        eprintln!("ait-server configuration failed:\n{message}");
        process::exit(78);
    });
    let runtime_config =
        ServerBinaryLifecycleConfig::from_server_data_root(prepared.data_root.clone())
            .unwrap_or_else(|message| {
                eprintln!("ait-server runtime configuration failed:\n{message}");
                process::exit(78);
            });

    match prepared.command {
        LifecycleCommand::Init => {
            initialize(&prepared, &runtime_config);
            return;
        }
        LifecycleCommand::Probe => {
            print_startup_probe(&prepared);
            return;
        }
        LifecycleCommand::Run => {}
        LifecycleCommand::Help | LifecycleCommand::Version => unreachable!(),
    }
    if prepared.init_if_missing {
        initialize(&prepared, &runtime_config);
    }

    ensure_startup_runtime_access(&prepared.data_root, prepared.defer_ci_admission).unwrap_or_else(
        |message| {
            eprintln!("ait-server startup probe failed:\n{message}");
            process::exit(78);
        },
    );
    serve(prepared.listen_address, runtime_config).await;
}

fn initialize(prepared: &PreparedLifecycle, config: &ServerBinaryLifecycleConfig) {
    ensure_durable_runtime_access(&prepared.data_root).unwrap_or_else(|message| {
        eprintln!("ait-server durable runtime probe failed:\n{message}");
        process::exit(78);
    });
    let report = initialize_installed_runtime(config).unwrap_or_else(|message| {
        eprintln!("ait-server initialization failed:\n{message}");
        process::exit(78);
    });
    println!(
        "ait-server initialization {}: runtime_root={} data_root_source={} activation_pointer={} generation_root={}",
        if report.created { "created" } else { "existing" },
        report.runtime_root.display(),
        prepared.data_root_source,
        report.activation_pointer.display(),
        report.generation_root.display(),
    );
}

fn print_startup_probe(prepared: &PreparedLifecycle) {
    let report = ensure_startup_runtime_access(&prepared.data_root, prepared.defer_ci_admission)
        .unwrap_or_else(|message| {
            eprintln!("ait-server startup probe failed:\n{message}");
            process::exit(78);
        });
    let ci_root = if report.ci_startup_admission_deferred {
        "deferred".to_string()
    } else {
        report.ci_ram_runtime_root.display().to_string()
    };
    println!(
        "ait-server startup probe ok: runtime_root={} data_root_source={} ci_ram_runtime_root={} ci_ram_runtime_root_source={} ci_runtime_pruned_run_base_count={} object_probe={}",
        report.runtime_root.display(),
        prepared.data_root_source,
        ci_root,
        report.ci_ram_runtime_root_source,
        report.ci_runtime_pruned_run_base_count,
        report.object_probe
    );
    if let Some(hint) = report.launch_hint {
        println!("ait-server startup probe hint: {hint}");
    }
}

async fn serve(address: SocketAddr, runtime_config: ServerBinaryLifecycleConfig) {
    let server = Server::try_bind(&address)
        .unwrap_or_else(|exc| panic!("ait-server failed to bind {address}: {exc}"));
    println!("ait-server bound on {address}; validating Binary DB registry");
    let (startup_app, startup_handle) = build_startup_router();
    let mut serving = Box::pin(server.serve(startup_app.into_make_service()));
    let mut loading = tokio::task::spawn_blocking(move || build_router(runtime_config));
    let app = tokio::select! {
        loaded = &mut loading => match loaded {
            Ok(app) => app,
            Err(error) => {
                eprintln!("ait-server Binary DB registry startup failed: {error}");
                process::exit(78);
            }
        },
        result = &mut serving => {
            result.unwrap_or_else(|exc| panic!("ait-server failed while registry was loading: {exc}"));
            panic!("ait-server stopped while Binary DB registry was loading");
        }
    };
    startup_handle
        .activate(app)
        .unwrap_or_else(|exc| panic!("ait-server failed to activate release router: {exc}"));
    println!("ait-server registry ready; serving on {address}");
    serving
        .await
        .unwrap_or_else(|exc| panic!("ait-server failed to start: {exc}"));
}
