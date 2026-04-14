use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use gl::events::GardenEventBuilder;
use gl::seed::Garden;

fn build_manifest(container_count: usize) -> String {
    let mut yaml = String::from(
        "apiVersion: v0\nkind: Garden\nmeta:\n  name: bench-garden\n  id: bench-1\nnet:\n  preset: bridge\ncontainers:\n",
    );

    for idx in 0..container_count {
        yaml.push_str(&format!(
            "  - name: c{idx}\n    rootfs:\n      path: /tmp/rootfs\n    entrypoint:\n      cmd: [\"/bin/sh\", \"-c\", \"echo hi\"]\n    limits:\n      cpu:\n        shares: 128\n      memory:\n        max: \"64Mi\"\n      pids:\n        max: 32\n"
        ));
    }

    yaml
}

fn bench_manifest_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("gl_manifest_parse");

    for container_count in [1usize, 4, 8, 16] {
        let yaml = build_manifest(container_count);

        group.bench_with_input(
            BenchmarkId::from_parameter(container_count),
            &container_count,
            |b, _| {
                b.iter(|| {
                    let garden: Garden = serde_yaml::from_str(black_box(&yaml)).unwrap();
                    black_box(garden);
                });
            },
        );
    }

    group.finish();
}

fn bench_event_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("gl_event_serialization");

    for event_count in [1usize, 16, 64, 256] {
        let builder = GardenEventBuilder::new("run-1".to_string(), "garden-1".to_string());

        group.bench_with_input(
            BenchmarkId::from_parameter(event_count),
            &event_count,
            |b, &count| {
                b.iter(|| {
                    let events: Vec<_> = (0..count)
                        .map(|idx| builder.container_start(&format!("c{idx}"), 1000 + idx as i32))
                        .collect();
                    let json = serde_json::to_vec(black_box(&events)).unwrap();
                    black_box(json);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_manifest_parse, bench_event_serialization);
criterion_main!(benches);
