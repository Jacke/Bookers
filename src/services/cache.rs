use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use chrono::{DateTime, Utc, Duration};

/// Cache entry with expiration
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry<T> {
    value: T,
    created_at: DateTime<Utc>,
    ttl_seconds: i64,
}

impl<T> CacheEntry<T> {
    fn is_expired(&self) -> bool {
        Utc::now() > self.created_at + Duration::seconds(self.ttl_seconds)
    }
}

/// In-memory cache with TTL
#[derive(Clone)]
pub struct TimedCache<K, V> {
    data: Arc<RwLock<HashMap<K, CacheEntry<V>>>>,
    default_ttl_seconds: i64,
}

impl<K, V> TimedCache<K, V>
where
    K: std::hash::Hash + Eq + Clone,
    V: Clone,
{
    pub fn new(default_ttl_seconds: i64) -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            default_ttl_seconds,
        }
    }
    
    pub async fn get(&self, key: &K) -> Option<V> {
        let data = self.data.read().await;
        
        if let Some(entry) = data.get(key) {
            if !entry.is_expired() {
                return Some(entry.value.clone());
            }
        }
        
        None
    }
    
    pub async fn set(&self, key: K, value: V) {
        self.set_with_ttl(key, value, self.default_ttl_seconds).await;
    }
    
    pub async fn set_with_ttl(&self, key: K, value: V, ttl_seconds: i64) {
        let mut data = self.data.write().await;
        data.insert(key, CacheEntry {
            value,
            created_at: Utc::now(),
            ttl_seconds,
        });
    }
    
    pub async fn remove(&self, key: &K) {
        let mut data = self.data.write().await;
        data.remove(key);
    }
    
    pub async fn clear(&self) {
        let mut data = self.data.write().await;
        data.clear();
    }
    
    /// Clean up expired entries
    pub async fn cleanup(&self) {
        let mut data = self.data.write().await;
        data.retain(|_, entry| !entry.is_expired());
    }
    
    pub async fn len(&self) -> usize {
        let data = self.data.read().await;
        data.len()
    }
    
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

/// AI Parse cache - caches parsed OCR results by content hash
#[derive(Clone)]
pub struct AIParseCache {
    cache: TimedCache<String, crate::services::ai_parser::AIParseResult>,
}

impl AIParseCache {
    /// Default TTL: 7 days (AI parsing is expensive and results don't change)
    const DEFAULT_TTL: i64 = 7 * 24 * 60 * 60;
    
    pub fn new() -> Self {
        Self {
            cache: TimedCache::new(Self::DEFAULT_TTL),
        }
    }
    
    /// Generate hash key from OCR text
    fn generate_key(text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        format!("{:x}", hasher.finalize())
    }
    
    pub async fn get(&self, ocr_text: &str) -> Option<crate::services::ai_parser::AIParseResult> {
        let key = Self::generate_key(ocr_text);
        self.cache.get(&key).await
    }
    
    pub async fn set(&self, ocr_text: &str, result: crate::services::ai_parser::AIParseResult) {
        let key = Self::generate_key(ocr_text);
        self.cache.set(key, result).await;
    }
    
    pub async fn cleanup(&self) {
        self.cache.cleanup().await;
    }
}

impl Default for AIParseCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Formula search index - caches search results
#[derive(Clone)]
pub struct FormulaSearchCache {
    cache: TimedCache<String, Vec<crate::models::Problem>>,
}

impl FormulaSearchCache {
    /// Default TTL: 1 hour (search results may change as data grows)
    const DEFAULT_TTL: i64 = 60 * 60;
    
    pub fn new() -> Self {
        Self {
            cache: TimedCache::new(Self::DEFAULT_TTL),
        }
    }
    
    pub async fn get(&self, query: &str) -> Option<Vec<crate::models::Problem>> {
        let key = query.to_lowercase().trim().to_string();
        self.cache.get(&key).await
    }
    
    pub async fn set(&self, query: &str, results: Vec<crate::models::Problem>) {
        let key = query.to_lowercase().trim().to_string();
        self.cache.set(key, results).await;
    }
    
    pub async fn invalidate(&self) {
        self.cache.clear().await;
    }
}

impl Default for FormulaSearchCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Export result cache - caches generated exports
#[derive(Clone)]
pub struct ExportCache {
    cache: TimedCache<String, Vec<u8>>, // key -> file bytes
}

impl ExportCache {
    /// Default TTL: 24 hours (exports are large and don't change often)
    const DEFAULT_TTL: i64 = 24 * 60 * 60;
    
    pub fn new() -> Self {
        Self {
            cache: TimedCache::new(Self::DEFAULT_TTL),
        }
    }
    
    pub async fn get(&self, book_id: &str, format: &str) -> Option<Vec<u8>> {
        let key = format!("{}:{}", book_id, format);
        self.cache.get(&key).await
    }
    
    pub async fn set(&self, book_id: &str, format: &str, data: Vec<u8>) {
        let key = format!("{}:{}", book_id, format);
        self.cache.set(key, data).await;
    }
    
    pub async fn invalidate_book(&self, _book_id: &str) {
        // Clear all formats for this book
        self.cache.clear().await;
    }
}

impl Default for ExportCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_timed_cache() {
        let cache: TimedCache<String, String> = TimedCache::new(1); // 1 second TTL
        
        cache.set("key1".to_string(), "value1".to_string()).await;
        
        assert_eq!(cache.get(&"key1".to_string()).await, Some("value1".to_string()));
        
        // Wait for expiration
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        assert_eq!(cache.get(&"key1".to_string()).await, None);
    }
    
    #[test]
    fn test_hash_generation() {
        let text1 = "Задача 15. Решите уравнение $x^2 = 4$";
        let text2 = "Задача 15. Решите уравнение $x^2 = 4$";
        let text3 = "Другой текст";
        
        assert_eq!(
            AIParseCache::generate_key(text1),
            AIParseCache::generate_key(text2)
        );
        
        assert_ne!(
            AIParseCache::generate_key(text1),
            AIParseCache::generate_key(text3)
        );
    }
}
