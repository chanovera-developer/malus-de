use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Inicializa el sistema unificado de logs estructurados con tracing y registra el hook de panics
pub fn init_logging(crate_name: &str) {
    crate::crash::install_panic_hook(crate_name);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("{crate_name}=debug,malus_de_common=debug,info"))
    });

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();
}
