use std::{env, fs::File, sync::Arc};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use web_push::{
    ContentEncoding, IsahcWebPushClient, PartialVapidSignatureBuilder, SubscriptionInfo,
    VapidSignatureBuilder, WebPushClient, WebPushMessageBuilder,
};

#[derive(Clone)]
pub struct PushService {
    client: Arc<IsahcWebPushClient>,
    vapid: PartialVapidSignatureBuilder,
    public_key: String,
    subject: String,
}

#[derive(Debug)]
pub struct SubscriptionRecord {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

#[derive(Serialize)]
pub struct NotificationPayload<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub conversation_id: &'a str,
    pub message_id: &'a str,
}

impl PushService {
    pub fn from_environment() -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let Ok(private_key_path) = env::var("VAPID_PRIVATE_KEY_PATH") else {
            return Ok(None);
        };

        let vapid = VapidSignatureBuilder::from_pem_no_sub(File::open(private_key_path)?)?;
        let public_key = URL_SAFE_NO_PAD.encode(vapid.get_public_key());
        let subject =
            env::var("VAPID_SUBJECT").unwrap_or_else(|_| "mailto:admin@cassy.local".to_string());

        Ok(Some(Self {
            client: Arc::new(IsahcWebPushClient::new()?),
            vapid,
            public_key,
            subject,
        }))
    }

    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    pub async fn send(
        &self,
        subscription: &SubscriptionRecord,
        payload: &NotificationPayload<'_>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let subscription = SubscriptionInfo::new(
            subscription.endpoint.clone(),
            subscription.p256dh.clone(),
            subscription.auth.clone(),
        );

        let mut signature_builder = self.vapid.clone().add_sub_info(&subscription);
        signature_builder.add_claim("sub", self.subject.clone());
        let signature = signature_builder.build()?;
        let encoded_payload = serde_json::to_vec(payload)?;

        let mut message = WebPushMessageBuilder::new(&subscription);
        message.set_payload(ContentEncoding::Aes128Gcm, &encoded_payload);
        message.set_vapid_signature(signature);
        message.set_ttl(60);

        self.client.send(message.build()?).await?;
        Ok(())
    }
}
