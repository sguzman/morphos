use criterion::{Criterion, criterion_group, criterion_main};
use geom_geometry::{BoolmeshBackend, GeometryEvaluator};
use geom_scene::{NodeId, SceneSource, parse_scene};
use std::fs;
use std::path::Path;

fn benchmark_scene_source() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("examples")
            .join("scenes")
            .join("benchmark-cache-tree.toml"),
    )
    .expect("read benchmark scene")
}

fn evaluation_benchmarks(criterion: &mut Criterion) {
    let source = benchmark_scene_source();
    let scene = parse_scene(&source).expect("parse benchmark scene");
    let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());

    criterion.bench_function("geometry/cold_root", |bench| {
        bench.iter(|| {
            let mut evaluator = GeometryEvaluator::new(BoolmeshBackend::new());
            evaluator.evaluate_root(&scene).expect("cold evaluation");
        });
    });

    evaluator.evaluate_root(&scene).expect("prime cache");
    criterion.bench_function("geometry/warm_root", |bench| {
        bench.iter(|| {
            evaluator.evaluate_root(&scene).expect("warm evaluation");
        });
    });

    let mut scene_source = SceneSource::parse(&source).expect("parse source");
    let updated = scene_source
        .set_parameter_scalar(
            &geom_scene::ParamId::new("left_width").expect("param id"),
            1.75,
        )
        .expect("edit scene");
    let selected = NodeId::new("root").expect("root id");
    criterion.bench_function("geometry/local_change_root", |bench| {
        bench.iter(|| {
            evaluator
                .evaluate_node(&updated, &selected)
                .expect("local change evaluation");
        });
    });
}

criterion_group!(benches, evaluation_benchmarks);
criterion_main!(benches);
