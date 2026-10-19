//! Cache preparation must refuse before network admission when no embedding
//! composition has started the process-scoped network service. This is a
//! dedicated integration-test binary so its process-global service starts
//! uninitialized and is not affected by other integration tests.

use std::path::Path;

use host_play::cache::{self, CachePreparationError};
use host_play::progress::ProfileProgressObserver;

#[test]
fn cache_prepare_reports_service_not_started_before_network_admission() {
    let cache = Path::new("/cache-service-refusal/cache");
    let unpack = Path::new("/cache-service-refusal/unpack");

    let result = cache::prepare(
        cache,
        unpack,
        client::Transport::Tcp,
        "127.0.0.1",
        1,
        &ProfileProgressObserver::default(),
    );

    assert_eq!(
        result,
        Err(CachePreparationError::ServiceStart(
            client_adapter::service::NetworkStartError::NotStarted,
        )),
    );
}
