//! Explicit local-only lifecycle smoke; installed weights are read, test data stays isolated.
use maru_core::{Config, Paths, live_model::LiveModel};
use maru_engine::{BootOptions, boot};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "Usage: model_control_smoke INSTALLED_CONFIG INSTALLED_DATA ISOLATED_HOME".into(),
        );
    }
    let installed = PathBuf::from(&args[1]);
    let mut paths = Paths::under(&PathBuf::from(&args[2]));
    paths.models_dir = installed.join("models");
    paths.bin_dir = installed.join("bin");
    let mut cfg = Config::load(std::path::Path::new(&args[0]))?;
    cfg.continuous.translation.model = LiveModel::HyMt2Small;
    cfg.continuous.translation.hy_mt2_small_path = maru_engine::live_models::available(&paths)
        .into_iter()
        .find(|m| m.model == "hy_mt2_small")
        .and_then(|m| m.path);
    if cfg.continuous.translation.hy_mt2_small_path.is_none() {
        return Err("The approved small HY GGUF is missing".into());
    }
    cfg.continuous.cpu_ocr = false;
    cfg.continuous.translation.dialogue_context = false;
    let services = boot(
        paths,
        cfg,
        BootOptions {
            defer_models: true,
            ..Default::default()
        },
    )
    .await;
    println!(
        "{}",
        serde_json::json!({"phase":"idle","notes":services.notes,"targets":services.resource_targets()})
    );
    for run in 1..=2 {
        let start = std::time::Instant::now();
        services.prepare_live_session().await?;
        println!(
            "{}",
            serde_json::json!({"phase":"prepared","run":run,"took_ms":start.elapsed().as_millis(),"status":services.live_setup_status(),"targets":services.resource_targets()})
        );
        if services.live_setup_status().preparation != "Prepared" {
            return Err("Preparation did not reach ready".into());
        }
        if !services.engine.history().is_empty() {
            return Err("Preparation polluted history".into());
        }
        let voice = services.speech_lease().await?;
        let before = services.resource_targets();
        services.engine.set_live_active(true);
        let message = services.release_idle_models().await?;
        let after = services.resource_targets();
        if services.live_setup_status().preparation != "Prepared" {
            return Err("Releasing unrelated models cleared live readiness".into());
        }
        for row in before.iter().filter(|r| r.role != "app" && r.pid.is_some()) {
            if after
                .iter()
                .find(|r| r.role == row.role)
                .and_then(|r| r.pid)
                != row.pid
            {
                return Err(format!("Busy {} was released", row.role).into());
            }
        }
        println!(
            "{}",
            serde_json::json!({"phase":"busy_preserved","run":run,"message":message,"targets":after})
        );
        drop(voice);
        println!(
            "{}",
            serde_json::json!({"phase":"voice_released","run":run,"message":services.release_idle_models().await?,"targets":services.resource_targets()})
        );
        services.engine.set_live_active(false);
        let message = services.release_idle_models().await?;
        println!(
            "{}",
            serde_json::json!({"phase":"released","run":run,"message":message,"targets":services.resource_targets()})
        );
        if services
            .resource_targets()
            .iter()
            .any(|r| r.role != "app" && r.pid.is_some())
        {
            return Err("An owned model remained after release".into());
        }
        if !services.engine.history().is_empty() {
            return Err("Model controls polluted history".into());
        }
        if services.live_setup_status().preparation == "Prepared" {
            return Err("Released models still report prepared".into());
        }
    }
    Ok(())
}
