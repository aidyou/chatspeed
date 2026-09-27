#![allow(clippy::module_inception)]

pub mod budget {
    pub mod errors;
    pub mod types;
}

pub mod workflow {
    pub mod react {
        pub mod experiment;
        pub mod campaign;
        pub mod experiment_schedule {
            pub mod fixture;
            pub mod types;
        }
        pub mod experiment_promotion {
            pub mod policy;
            pub mod types;
        }
    }
}

pub use workflow::react::campaign;
pub use workflow::react::experiment;
pub use workflow::react::experiment_promotion;
pub use workflow::react::experiment_schedule;
