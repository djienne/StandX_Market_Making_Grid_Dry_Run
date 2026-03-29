//! Rolling window statistics with O(1) incremental updates.
//!
//! Copied verbatim from standx. Pure math, zero external dependencies.

#[derive(Debug)]
pub struct RollingWindow {
    buffer: Box<[f64]>,
    capacity: usize,
    write_pos: usize,
    count: usize,
}

impl RollingWindow {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be greater than 0");
        Self {
            buffer: vec![0.0; capacity].into_boxed_slice(),
            capacity,
            write_pos: 0,
            count: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, value: f64) -> Option<f64> {
        let idx = self.write_pos % self.capacity;
        let old_value = if self.count >= self.capacity { Some(self.buffer[idx]) } else { None };
        self.buffer[idx] = value;
        self.write_pos = self.write_pos.wrapping_add(1);
        if self.count < self.capacity { self.count += 1; }
        old_value
    }

    #[inline]
    pub fn len(&self) -> usize { self.count }

    #[inline]
    pub fn is_empty(&self) -> bool { self.count == 0 }

    #[inline]
    pub fn is_full(&self) -> bool { self.count >= self.capacity }

    #[inline]
    pub fn capacity(&self) -> usize { self.capacity }

    pub fn clear(&mut self) {
        self.write_pos = 0;
        self.count = 0;
    }
}

#[derive(Debug)]
pub struct RollingStats {
    window: RollingWindow,
    sum: f64,
    sum_sq: f64,
    cached_std: f64,
    cached_mean: f64,
}

impl RollingStats {
    pub fn new(capacity: usize) -> Self {
        Self {
            window: RollingWindow::new(capacity),
            sum: 0.0,
            sum_sq: 0.0,
            cached_std: 0.0,
            cached_mean: 0.0,
        }
    }

    #[inline]
    pub fn push(&mut self, value: f64) {
        if !value.is_finite() { return; }
        let old_value = self.window.push(value);
        self.sum += value;
        self.sum_sq += value * value;
        if let Some(old_val) = old_value {
            self.sum -= old_val;
            self.sum_sq -= old_val * old_val;
        }
        let n = self.window.len();
        if n >= 2 {
            self.cached_mean = self.sum / n as f64;
            let mean_sq = self.sum_sq / n as f64;
            let variance = (mean_sq - self.cached_mean * self.cached_mean).max(0.0);
            self.cached_std = variance.sqrt();
        } else if n == 1 {
            self.cached_mean = self.sum;
            self.cached_std = 0.0;
        }
    }

    #[inline]
    pub fn mean(&self) -> f64 {
        if self.window.len() == 0 { 0.0 } else { self.sum / self.window.len() as f64 }
    }

    #[inline]
    pub fn std(&self) -> f64 {
        let n = self.window.len();
        if n < 2 { return 0.0; }
        let mean = self.sum / n as f64;
        let mean_sq = self.sum_sq / n as f64;
        (mean_sq - mean * mean).max(0.0).sqrt()
    }

    #[inline]
    pub fn zscore(&self, value: f64) -> f64 {
        if self.cached_std < 1e-10 { return 0.0; }
        (value - self.cached_mean) / self.cached_std
    }

    #[inline]
    pub fn len(&self) -> usize { self.window.len() }

    #[inline]
    pub fn is_empty(&self) -> bool { self.window.is_empty() }

    pub fn clear(&mut self) {
        self.window.clear();
        self.sum = 0.0;
        self.sum_sq = 0.0;
        self.cached_std = 0.0;
        self.cached_mean = 0.0;
    }
}
