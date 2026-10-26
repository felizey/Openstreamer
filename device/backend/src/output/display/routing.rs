/// Match connectors to independent controllers/planes. A greedy assignment
/// can consume the only controller supported by the second HDMI port.
pub(super) fn distinct<T: Copy + Eq>(candidates: &[Vec<T>]) -> Option<Vec<T>> {
    fn assign<T: Copy + Eq>(remaining: &[Vec<T>], chosen: &mut Vec<T>) -> bool {
        let Some((first, rest)) = remaining.split_first() else {
            return true;
        };
        for &candidate in first {
            if chosen.contains(&candidate) {
                continue;
            }
            chosen.push(candidate);
            if assign(rest, chosen) {
                return true;
            }
            chosen.pop();
        }
        false
    }
    let mut chosen = Vec::new();
    assign(candidates, &mut chosen).then_some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avoids_stealing_the_second_ports_only_controller() {
        assert_eq!(distinct(&[vec![1, 2], vec![1]]), Some(vec![2, 1]));
    }

    #[test]
    fn preserves_current_routes_when_they_are_independent() {
        assert_eq!(distinct(&[vec![2, 1], vec![1, 2]]), Some(vec![2, 1]));
    }

    #[test]
    fn refuses_shared_or_missing_resources() {
        assert_eq!(distinct(&[vec![1], vec![1]]), None);
        assert_eq!(distinct(&[vec![1], vec![]]), None);
        assert_eq!(distinct(&[vec![2]]), Some(vec![2]));
    }
}
