use crate::*;
use lair_keystore_api::prelude::*;

pub struct Lair {
    _keystore: lair_keystore_api::in_proc_keystore::InProcKeystore,
    pub lair_client: LairClient,
}

pub async fn load(config: Arc<config::Config>) -> Result<Lair> {
    tracing::info!("Starting in-process lair instance...");

    let store_factory =
        lair_keystore::create_sql_pool_factory(config.lair_config.store_file.clone());

    let passphrase = sodoken::BufRead::new_no_lock(config.lair_passphrase.as_bytes());

    let keystore = PwHashLimits::Interactive
        .with_exec(|| {
            lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                config.lair_config.clone(),
                store_factory,
                passphrase,
            )
        })
        .await?;

    let lair_client = keystore.new_client().await?;

    let mut tag_ok = false;

    if let Ok(LairEntryInfo::Seed { .. }) = lair_client.get_entry(config.lair_tag.clone()).await {
        tag_ok = true;
    }

    if !tag_ok {
        lair_client
            .new_seed(config.lair_tag.clone(), None, false)
            .await?;
    }

    Ok(Lair {
        _keystore: keystore,
        lair_client,
    })
}
