use std::sync::Arc;

use poise::serenity_prelude::{Cache, Http, Result};
use surrealdb::{Surreal, engine::remote::ws};
use tokio::sync::Mutex;

use super::{autodelete::AutoDeleteHandler, handler::DiscordEventHandler};
use crate::{
    MuniBotError,
    config::{Config, DiscordConfig},
    handlers::{DiscordMessageHandlerCollection, logging::LoggingHandler},
};

#[derive(Clone, Debug)]
pub struct GlobalAccess {
    db: Arc<Surreal<ws::Client>>,
    http: Arc<Http>,
    cache: Arc<Cache>,
}

impl GlobalAccess {
    pub fn new(http: Arc<Http>, cache: Arc<Cache>, db: Arc<Surreal<ws::Client>>) -> Self {
        Self { http, cache, db }
    }

    pub fn db(&self) -> &Surreal<ws::Client> {
        &self.db
    }

    pub fn http(&self) -> &Http {
        &self.http
    }

    pub fn cache(&self) -> &Arc<Cache> {
        &self.cache
    }

    pub fn as_cache_http(&self) -> (&Arc<Cache>, &Http) {
        (&self.cache, &*self.http)
    }
}

pub struct DiscordState {
    pub config: DiscordConfig,
    handlers: DiscordMessageHandlerCollection,
    access: GlobalAccess,

    logging: Arc<Mutex<LoggingHandler>>,
    autodeletion: Arc<Mutex<AutoDeleteHandler>>,
}
impl DiscordState {
    /// creates a new `DiscordState` struct. the `LoggingHandler` and
    /// `AutoDeleteHandler` are added for you.
    pub async fn new(
        mut handlers: DiscordMessageHandlerCollection,
        config: &Config,
        db: Arc<Surreal<ws::Client>>,
        http: Arc<Http>,
        cache: Arc<Cache>,
    ) -> Result<Self, MuniBotError> {
        let global_access = GlobalAccess { db, http, cache };

        // add the logging handler to the list of handlers
        let logging = Arc::new(Mutex::new(LoggingHandler::new(global_access.clone())));

        handlers.push(logging.clone());

        // autodeletion handler
        let autodeletion = Arc::new(Mutex::new(
            AutoDeleteHandler::new(global_access.clone(), logging.clone()).await?,
        ));

        Ok(Self {
            handlers,
            config: config.discord.clone(),
            access: global_access,
            logging,
            autodeletion,
        })
    }

    pub fn handlers(&self) -> &[Arc<Mutex<dyn DiscordEventHandler>>] {
        &self.handlers
    }

    pub fn access(&self) -> &GlobalAccess {
        &self.access
    }

    pub fn logging(&self) -> &Arc<Mutex<LoggingHandler>> {
        &self.logging
    }

    pub fn autodeletion(&self) -> &Arc<Mutex<AutoDeleteHandler>> {
        &self.autodeletion
    }
}
