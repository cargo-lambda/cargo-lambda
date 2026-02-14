use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

pub type InstanceId = Uuid;

#[derive(Clone, Debug, PartialEq)]
pub enum InstanceStatus {
    Starting,
    Idle,
    Busy,
}

#[derive(Clone, Debug)]
pub struct FunctionInstance {
    pub id: InstanceId,
    pub status: InstanceStatus,
    pub requests_processed: u64,
}

impl FunctionInstance {
    pub fn new(id: InstanceId) -> Self {
        Self {
            id,
            status: InstanceStatus::Starting,
            requests_processed: 0,
        }
    }
}

#[derive(Clone)]
pub struct InstancePool {
    instances: Arc<RwLock<HashMap<InstanceId, FunctionInstance>>>,
    max_concurrency: usize,
}

impl InstancePool {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
            max_concurrency,
        }
    }

    /// Returns true if a new instance should be spawned based on queue depth and current capacity
    pub async fn should_spawn_instance(&self, queue_depth: usize) -> bool {
        if queue_depth == 0 {
            return false;
        }

        let instances = self.instances.read().await;
        let current_count = instances.len();

        if current_count >= self.max_concurrency {
            return false;
        }

        let idle_count = instances
            .values()
            .filter(|inst| inst.status == InstanceStatus::Idle)
            .count();

        idle_count == 0
    }

    /// Mark instance as busy (processing a request)
    pub async fn mark_busy(&self, instance_id: &InstanceId) {
        let mut instances = self.instances.write().await;
        if let Some(instance) = instances.get_mut(instance_id) {
            instance.status = InstanceStatus::Busy;
            instance.requests_processed += 1;
        }
    }

    /// Mark instance as idle (waiting for requests)
    pub async fn mark_idle(&self, instance_id: &InstanceId) {
        let mut instances = self.instances.write().await;
        if let Some(instance) = instances.get_mut(instance_id) {
            instance.status = InstanceStatus::Idle;
        }
    }

    /// Add a new instance to the pool
    pub async fn add_instance(&self, instance: FunctionInstance) {
        let mut instances = self.instances.write().await;
        instances.insert(instance.id, instance);
    }

    /// Remove an instance from the pool (e.g., crashed or dead)
    pub async fn remove_instance(&self, instance_id: &InstanceId) -> Option<FunctionInstance> {
        let mut instances = self.instances.write().await;
        instances.remove(instance_id)
    }

    /// Get the current count of instances
    pub async fn instance_count(&self) -> usize {
        let instances = self.instances.read().await;
        instances.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_should_spawn_instance_first_request() {
        let pool = InstancePool::new(3);
        assert!(!pool.should_spawn_instance(0).await);
        assert!(pool.should_spawn_instance(1).await);
    }

    #[tokio::test]
    async fn test_should_spawn_instance_at_max_capacity() {
        let pool = InstancePool::new(2);

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        pool.add_instance(FunctionInstance::new(id1)).await;
        pool.add_instance(FunctionInstance::new(id2)).await;

        assert!(!pool.should_spawn_instance(5).await);
    }

    #[tokio::test]
    async fn test_should_spawn_instance_with_idle() {
        let pool = InstancePool::new(3);

        let id = Uuid::new_v4();
        let mut instance = FunctionInstance::new(id);
        instance.status = InstanceStatus::Idle;
        pool.add_instance(instance).await;

        assert!(!pool.should_spawn_instance(1).await);
    }

    #[tokio::test]
    async fn test_should_spawn_instance_all_busy() {
        let pool = InstancePool::new(3);

        let id = Uuid::new_v4();
        let mut instance = FunctionInstance::new(id);
        instance.status = InstanceStatus::Busy;
        pool.add_instance(instance).await;

        assert!(pool.should_spawn_instance(1).await);
    }

    #[tokio::test]
    async fn test_mark_busy_and_idle() {
        let pool = InstancePool::new(3);
        let id = Uuid::new_v4();

        pool.add_instance(FunctionInstance::new(id)).await;

        pool.mark_idle(&id).await;
        {
            let instances = pool.instances.read().await;
            let instance = instances.get(&id).unwrap();
            assert_eq!(instance.status, InstanceStatus::Idle);
        }

        pool.mark_busy(&id).await;
        {
            let instances = pool.instances.read().await;
            let instance = instances.get(&id).unwrap();
            assert_eq!(instance.status, InstanceStatus::Busy);
            assert_eq!(instance.requests_processed, 1);
        }
    }

    #[tokio::test]
    async fn test_remove_instance() {
        let pool = InstancePool::new(3);
        let id = Uuid::new_v4();

        pool.add_instance(FunctionInstance::new(id)).await;
        assert_eq!(pool.instance_count().await, 1);

        let removed = pool.remove_instance(&id).await;
        assert!(removed.is_some());
        assert_eq!(pool.instance_count().await, 0);
    }
}
