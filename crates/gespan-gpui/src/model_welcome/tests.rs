use super::*;

fn alpha(shape: &Shape) -> f32 {
    match shape {
        Shape::Disc { color, .. }
        | Shape::Ring { color, .. }
        | Shape::Line { color, .. }
        | Shape::Polygon { color, .. }
        | Shape::Rect { color, .. } => color.a,
    }
}

#[test]
fn opus_5_5_and_astra_are_recognized_without_their_siblings() {
    for names in [
        vec!["claude-opus-5-5"],
        vec!["Claude Opus 5.5"],
        vec!["anthropic/claude-opus-5-5-20260901"],
        vec!["new", "Opus 5.5"],
        vec!["opus5.5"],
    ] {
        assert_eq!(
            Kind::for_model(names.clone()),
            Some(Kind::Opus),
            "{names:?}"
        );
    }
    for names in [
        vec!["claude-opus-5"],
        vec!["Opus 5.6"],
        vec!["claude-opus-5-55"],
        vec!["Opus 5.5.1"],
        vec!["magnumopus-5-5"],
    ] {
        assert_eq!(Kind::for_model(names.clone()), None, "{names:?}");
    }
    for names in [vec!["gpt-6-astra"], vec!["ASTRA"], vec!["openai/astra"]] {
        assert_eq!(
            Kind::for_model(names.clone()),
            Some(Kind::Astra),
            "{names:?}"
        );
    }
    assert_eq!(Kind::for_model(["Astral"]), None);
    assert_eq!(Kind::for_model(["gpt-6-astra2"]), None);
}

#[test]
fn the_opus_stage_sits_under_a_centered_composer_or_above_a_docked_one() {
    assert_eq!(opus_stage(800., Some((360., 480.))), (480., 320.));
    assert_eq!(opus_stage(800., Some((680., 780.))), (440., 240.));
    assert_eq!(opus_stage(200., Some((100., 190.))), (0., 100.));
    assert_eq!(opus_stage(800., None), (440., 360.));
}

#[test]
fn each_scene_fades_in_plays_and_is_gone_by_its_end() {
    for kind in [Kind::Opus, Kind::Astra] {
        let end = kind.duration().as_secs_f32();
        assert!(
            scene(kind, 0., 900., 700., None, false).is_empty(),
            "{kind:?} starts invisible"
        );
        assert!(
            scene(kind, end, 900., 700., None, false).is_empty(),
            "{kind:?} ends invisible"
        );
        assert!(scene(kind, end + 5., 900., 700., None, false).is_empty());
        let middle = scene(kind, end * 0.4, 900., 700., None, false);
        assert!(middle.len() > 20, "{kind:?} plays: {}", middle.len());
        assert!(middle.iter().all(|shape| alpha(shape) <= 1.));
    }
}

#[test]
fn the_opus_staff_writes_itself_and_notes_land_in_order() {
    let lines = |at: f32| {
        scene(Kind::Opus, at, 900., 700., Some((300., 420.)), false)
            .into_iter()
            .filter_map(|shape| match shape {
                Shape::Line { points, .. } => Some(points),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let early = lines(0.8);
    let later = lines(2.8);
    assert_eq!(later.len(), 5, "five staff lines");
    let reach = |lines: &[Vec<Point<f32>>]| {
        lines
            .iter()
            .map(|line| line.last().unwrap().x)
            .fold(0., f32::max)
    };
    assert!(reach(&early) < reach(&later), "the pen moves right");
    assert!(
        (reach(&later) - 900.).abs() < 1.,
        "and crosses the whole stage"
    );
    // The staff stays in the band under the composer.
    for point in later.iter().flatten() {
        assert!(point.y >= 420. - 30. && point.y <= 700., "{point:?}");
    }
    let notes = |at: f32| {
        scene(Kind::Opus, at, 900., 700., Some((300., 420.)), false)
            .into_iter()
            .filter(|shape| matches!(shape, Shape::Rect { .. }))
            .count()
    };
    assert_eq!(notes(1.0), 0, "no note before the first lands");
    assert!(notes(1.5) < notes(3.5), "more notes as the phrase plays");
    assert_eq!(notes(3.5), 20, "ten heads with their stems");
}

#[test]
fn astra_keeps_its_sun_above_the_composer_and_stars_fall_left() {
    let stars_at = |at: f32| {
        scene(Kind::Astra, at, 1200., 800., None, false)
            .into_iter()
            .filter_map(|shape| match shape {
                Shape::Line { points, width, .. } if points.len() == 2 && width == 2. => {
                    Some(points[0])
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // The first long star (index 0) is drawn from 0.35s.
    let (before, after) = (stars_at(0.6)[0], stars_at(1.2)[0]);
    assert!(
        after.x < before.x && after.y > before.y,
        "{before:?} → {after:?}"
    );
    // The sun's core is the brightest disc, top right and clear of the bottom.
    let core = scene(Kind::Astra, 1.5, 1200., 800., None, false)
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::Disc { center, radius, .. } if radius > 8. && radius < 20. => Some(center),
            _ => None,
        })
        .next()
        .expect("the sun");
    assert_eq!((core.x, core.y), (1200. * 0.82, 120.));
    let narrow = scene(Kind::Astra, 1.5, 300., 200., None, false);
    assert!(
        narrow
            .iter()
            .any(|shape| matches!(shape, Shape::Disc { center, .. } if center.y == 58.))
    );
}

#[test]
fn light_themes_use_darker_ink() {
    let ink = |light: bool| {
        scene(Kind::Opus, 3.5, 900., 700., None, light)
            .into_iter()
            .find_map(|shape| match shape {
                Shape::Rect { color, .. } => Some(color.l),
                _ => None,
            })
            .unwrap()
    };
    assert!(ink(true) < ink(false));
}
