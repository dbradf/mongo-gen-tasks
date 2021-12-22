use std::{collections::HashSet, path::PathBuf};

use shrub_rs::models::{
    project::EvgProject,
    task::{EvgTask, TaskRef},
    variant::{BuildVariant, DisplayTask},
};
use tokio::sync::mpsc;

use crate::{
    resmoke_task_gen::{ResmokeGenParams, ResmokeGenService},
    split_tasks::GeneratedSuite,
    task_types::fuzzer_tasks::FuzzerGenTaskParams,
};

#[derive(Clone, Debug)]
pub enum GeneratorMessage {
    ResmokeSuite(GeneratedSuite, ResmokeGenParams),
    FuzzerSuite(FuzzerGenTaskParams),
    GeneratorTasks(HashSet<String>),
    Write(String, PathBuf),
}
struct GeneratorActor {
    receiver: mpsc::Receiver<GeneratorMessage>,
    gen_task_def: Vec<EvgTask>,
    gen_task_specs: Vec<TaskRef>,
    display_tasks: Vec<DisplayTask>,
    resmoke_gen_service: ResmokeGenService,
}

impl GeneratorActor {
    fn new(receiver: mpsc::Receiver<GeneratorMessage>) -> Self {
        Self {
            gen_task_def: vec![],
            gen_task_specs: vec![],
            display_tasks: vec![],
            resmoke_gen_service: ResmokeGenService {},
            receiver,
        }
    }

    async fn run(&mut self) {
        while let Some(msg) = self.receiver.recv().await {
            self.handle_message(msg);
        }
    }

    fn handle_message(&mut self, msg: GeneratorMessage) {
        match msg {
            GeneratorMessage::ResmokeSuite(gen_suite, gen_params) => {
                self.resmoke_gen_service
                    .generate_tasks(&gen_suite, &gen_params)
                    .into_iter()
                    .for_each(|t| {
                        self.gen_task_def.push(t.clone());
                        self.gen_task_specs.push(t.get_reference(None, Some(false)));
                    });
                self.display_tasks.push(gen_suite.display_task());
            }
            GeneratorMessage::GeneratorTasks(found_tasks) => {
                self.display_tasks.push(DisplayTask {
                    name: "generator_tasks".to_string(),
                    execution_tasks: found_tasks.into_iter().collect(),
                });
            }
            GeneratorMessage::Write(bv_name, path) => {
                let gen_build_variant = BuildVariant {
                    name: bv_name.clone(),
                    tasks: self.gen_task_specs.clone(),
                    display_tasks: Some(self.display_tasks.clone()),
                    ..Default::default()
                };

                let gen_evg_project = EvgProject {
                    buildvariants: vec![gen_build_variant],
                    tasks: self.gen_task_def.clone(),
                    ..Default::default()
                };

                std::fs::write(path, serde_json::to_string(&gen_evg_project).unwrap()).unwrap();
            }
            GeneratorMessage::FuzzerSuite(params) => {
                // let generated_task = generate_fuzzer_task(&params);
                // self.gen_task_def.extend(generated_task.sub_tasks.clone());
                // self.gen_task_specs.extend(generated_task.build_task_ref());
                // self.display_tasks.push(generated_task.build_display_task());
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneratorActorHandle {
    sender: mpsc::Sender<GeneratorMessage>,
}

impl GeneratorActorHandle {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(64);
        let mut actor = GeneratorActor::new(receiver);
        tokio::spawn(async move { actor.run().await });

        Self { sender }
    }

    pub async fn generate_resmoke(&self, gen_suite: &GeneratedSuite, gen_params: ResmokeGenParams) {
        let msg = GeneratorMessage::ResmokeSuite(gen_suite.clone(), gen_params.clone());
        self.sender.send(msg).await.unwrap();
    }

    pub async fn generate_fuzzer(&self, fuzzer_params: FuzzerGenTaskParams) {
        let msg = GeneratorMessage::FuzzerSuite(fuzzer_params);
        self.sender.send(msg).await.unwrap();
    }

    pub async fn add_generator_tasks(&self, found_tasks: HashSet<String>) {
        let msg = GeneratorMessage::GeneratorTasks(found_tasks);
        self.sender.send(msg).await.unwrap();
    }

    pub async fn write(&self, bv_name: &str, path: PathBuf) {
        let msg = GeneratorMessage::Write(bv_name.to_string(), path);
        self.sender.send(msg).await.unwrap();
    }
}
