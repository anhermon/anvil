use std::collections::HashMap;

const K_FACTOR: f64 = 32.0;
const INITIAL_RATING: f64 = 1200.0;
const DIFFICULTY_DIVISOR: f64 = 400.0;

pub struct EloRating {
    ratings: HashMap<String, f64>,
    difficulties: HashMap<String, f64>,
}

impl Default for EloRating {
    fn default() -> Self {
        Self::new()
    }
}

impl EloRating {
    pub fn new() -> Self {
        Self {
            ratings: HashMap::new(),
            difficulties: HashMap::new(),
        }
    }

    pub fn get_rating(&self, task_name: &str) -> f64 {
        *self.ratings.get(task_name).unwrap_or(&INITIAL_RATING)
    }

    pub fn update(&mut self, task_name: &str, success: bool) {
        let rating = self
            .ratings
            .entry(task_name.to_string())
            .or_insert(INITIAL_RATING);
        let difficulty = self
            .difficulties
            .entry(task_name.to_string())
            .or_insert(INITIAL_RATING);

        let expected = expected_score(*rating, *difficulty);
        let actual = if success { 1.0 } else { 0.0 };

        let delta = K_FACTOR * (actual - expected);
        *rating += delta;

        let difficulty_delta = K_FACTOR * 0.1 * (actual - expected);
        *difficulty += difficulty_delta;
    }

    #[allow(dead_code)]
    pub fn average_rating(&self) -> f64 {
        if self.ratings.is_empty() {
            return INITIAL_RATING;
        }
        self.ratings.values().sum::<f64>() / self.ratings.len() as f64
    }

    #[allow(dead_code)]
    pub fn all_ratings(&self) -> &HashMap<String, f64> {
        &self.ratings
    }
}

fn expected_score(rating: f64, difficulty: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((difficulty - rating) / DIFFICULTY_DIVISOR))
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task_name: String,
    pub criteria_met: bool,
    pub elo_delta: f64,
}
