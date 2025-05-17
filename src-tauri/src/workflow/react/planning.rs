use crate::workflow::{
    error::WorkflowError,
    react::types::{Plan, PlanStatus},
};
use rust_i18n::t;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Plan manager for managing and executing plans
pub struct PlanManager {
    /// List of plans
    plans: Vec<Plan>,
    /// Next plan ID
    next_id: u32,
    /// Mutex lock for thread safety
    lock: Arc<Mutex<()>>,
}

impl PlanManager {
    /// Creates a new PlanManager instance
    ///
    /// # Returns
    /// A new PlanManager with empty plan list and initial ID of 1
    pub fn new() -> Self {
        Self {
            plans: Vec::new(),
            next_id: 1,
            lock: Arc::new(Mutex::new(())),
        }
    }

    /// Creates a new plan and adds it to the plan list
    ///
    /// # Arguments
    /// * `name` - Name of the plan
    /// * `goal` - Goal of the plan
    ///
    /// # Returns
    /// The newly created Plan or an error if creation fails
    pub async fn create_plan(&mut self, name: String, goal: String) -> Result<Plan, WorkflowError> {
        let _guard = self.lock.lock().await;

        let id = self.next_id;
        self.next_id += 1;

        let plan = Plan::new(id, name, goal);
        self.plans.push(plan.clone());

        Ok(plan)
    }

    /// Retrieves a plan by its ID
    ///
    /// # Arguments
    /// * `id` - ID of the plan to retrieve
    ///
    /// # Returns
    /// The requested Plan or an error if not found
    pub async fn get_plan(&self, id: u32) -> Result<Plan, WorkflowError> {
        let _guard = self.lock.lock().await;

        self.plans
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| {
                WorkflowError::Config(t!("workflow.react.plan_not_found", id = id).to_string())
            })
    }

    /// Retrieves all plans
    ///
    /// # Returns
    /// A vector containing all plans
    pub async fn get_all_plans(&self) -> Vec<Plan> {
        let _guard = self.lock.lock().await;
        self.plans.clone()
    }

    /// Retrieves the first pending or failed plan
    ///
    /// # Returns
    /// The first pending or failed plan, or None if no such plan exists
    pub async fn get_pending_plan(&self) -> Option<Plan> {
        let _guard = self.lock.lock().await;

        self.plans
            .iter()
            .find(|p| p.status == PlanStatus::Pending || p.status == PlanStatus::Failed)
            .cloned()
    }

    /// Updates an existing plan
    ///
    /// # Arguments
    /// * `plan` - The updated plan
    ///
    /// # Returns
    /// Ok if successful, or an error if the plan is not found
    pub async fn update_plan(&mut self, plan: Plan) -> Result<(), WorkflowError> {
        let _guard = self.lock.lock().await;

        if let Some(index) = self.plans.iter().position(|p| p.id == plan.id) {
            self.plans[index] = plan;
            Ok(())
        } else {
            Err(WorkflowError::Config(
                t!("workflow.react.plan_not_found", id = plan.id).to_string(),
            ))
        }
    }

    /// Updates the status of a plan
    ///
    /// # Arguments
    /// * `id` - ID of the plan to update
    /// * `status` - New status for the plan
    ///
    /// # Returns
    /// The updated Plan or an error if the plan is not found
    pub async fn update_plan_status(
        &mut self,
        id: u32,
        status: PlanStatus,
    ) -> Result<Plan, WorkflowError> {
        let _guard = self.lock.lock().await;

        if let Some(index) = self.plans.iter().position(|p| p.id == id) {
            self.plans[index].update_status(status);
            Ok(self.plans[index].clone())
        } else {
            Err(WorkflowError::Config(
                t!("workflow.react.plan_not_found", id = id).to_string(),
            ))
        }
    }

    /// Records an error for a plan
    ///
    /// # Arguments
    /// * `id` - ID of the plan to record error for
    /// * `error` - Error message to record
    ///
    /// # Returns
    /// The updated Plan or an error if the plan is not found
    pub async fn record_plan_error(
        &mut self,
        id: u32,
        error: String,
    ) -> Result<Plan, WorkflowError> {
        let _guard = self.lock.lock().await;

        if let Some(index) = self.plans.iter().position(|p| p.id == id) {
            self.plans[index].record_error(error);
            Ok(self.plans[index].clone())
        } else {
            Err(WorkflowError::Config(
                t!("workflow.react.plan_not_found", id = id).to_string(),
            ))
        }
    }

    /// Marks a plan as completed
    ///
    /// # Arguments
    /// * `id` - ID of the plan to complete
    /// * `summary` - Optional summary value to store with the plan
    ///
    /// # Returns
    /// The completed Plan or an error if the plan is not found
    pub async fn complete_plan(
        &mut self,
        id: u32,
        summary: Option<Value>,
    ) -> Result<Plan, WorkflowError> {
        let _guard = self.lock.lock().await;

        if let Some(index) = self.plans.iter().position(|p| p.id == id) {
            self.plans[index].complete(summary);
            Ok(self.plans[index].clone())
        } else {
            Err(WorkflowError::Config(
                t!("workflow.react.plan_not_found", id = id).to_string(),
            ))
        }
    }

    /// Resets a plan to its initial state
    ///
    /// # Arguments
    /// * `id` - ID of the plan to reset
    ///
    /// # Returns
    /// The reset Plan or an error if the plan is not found
    pub async fn reset_plan(&mut self, id: u32) -> Result<Plan, WorkflowError> {
        let _guard = self.lock.lock().await;

        if let Some(index) = self.plans.iter().position(|p| p.id == id) {
            self.plans[index].reset();
            Ok(self.plans[index].clone())
        } else {
            Err(WorkflowError::Config(
                t!("workflow.react.plan_not_found", id = id).to_string(),
            ))
        }
    }

    /// Deletes a plan
    ///
    /// # Arguments
    /// * `id` - ID of the plan to delete
    ///
    /// # Returns
    /// Ok if successful, or an error if the plan is not found
    pub async fn delete_plan(&mut self, id: u32) -> Result<(), WorkflowError> {
        let _guard = self.lock.lock().await;

        if let Some(index) = self.plans.iter().position(|p| p.id == id) {
            self.plans.remove(index);
            Ok(())
        } else {
            Err(WorkflowError::Config(
                t!("workflow.react.plan_not_found", id = id).to_string(),
            ))
        }
    }

    /// Clears all plans
    pub async fn clear_plans(&mut self) {
        let _guard = self.lock.lock().await;
        self.plans.clear();
    }
}
