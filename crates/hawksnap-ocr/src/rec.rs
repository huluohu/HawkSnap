//! 识别侧纯函数：CTC 贪心解码 + 标签表构建。角度分类（cls）与 CRNN 推理在
//! pipeline.rs（需要 ORT 会话），此处全部为可独立单测的纯逻辑。

/// rec 输出类别 → 字符的映射表：`labels[0]` = CTC blank（解码时跳过），随后是字典
/// 行；类别数比 blank+字典多 1 时（paddle `use_space_char=True`），末位为空格。
pub fn build_labels(dict_lines: Vec<String>, num_classes: usize) -> Vec<String> {
    let mut labels: Vec<String> = Vec::with_capacity(num_classes);
    labels.push(String::new()); // blank
    labels.extend(dict_lines);
    if num_classes == labels.len() + 1 {
        labels.push(" ".to_string());
    }
    labels
}

/// CTC 贪心解码：逐帧 argmax → 折叠重复 → 跳 blank → 映射字符 + 置信度（被采纳
/// 帧的概率均值）。`classes` 为 [T][C] 行优先展开。
pub fn ctc_greedy_decode(classes: &[f32], frames: usize, num_classes: usize, labels: &[String]) -> (String, f32) {
    if frames == 0 || num_classes == 0 || classes.len() < frames * num_classes {
        return (String::new(), 0.0);
    }
    let mut text = String::new();
    let mut conf_sum = 0f32;
    let mut conf_n = 0u32;
    let mut prev: usize = usize::MAX;
    for f in 0..frames {
        let row = &classes[f * num_classes..(f + 1) * num_classes];
        let mut best = 0usize;
        let mut best_p = f32::NEG_INFINITY;
        for (c, &p) in row.iter().enumerate() {
            if p > best_p {
                best_p = p;
                best = c;
            }
        }
        if best != 0 && best != prev {
            if let Some(ch) = labels.get(best) {
                text.push_str(ch);
                conf_sum += best_p;
                conf_n += 1;
            }
        }
        prev = best;
    }
    let score = if conf_n == 0 { 0.0 } else { conf_sum / conf_n as f32 };
    (text, score)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 序列 A A blank B → "AB"；重复帧折叠、blank 跳过。
    #[test]
    fn ctc_collapses_repeats_and_skips_blank() {
        let labels = vec!["".to_string(), "A".to_string(), "B".to_string()];
        // 3 类：blank/A/B。4 帧：A(0.9) A(0.8) blank(1.0) B(0.6)。
        let mut classes = Vec::new();
        for row in [[0.05, 0.9, 0.05], [0.1, 0.8, 0.1], [1.0, 0.0, 0.0], [0.3, 0.1, 0.6]] {
            classes.extend_from_slice(&row);
        }
        let (text, score) = ctc_greedy_decode(&classes, 4, 3, &labels);
        assert_eq!(text, "AB");
        assert!((score - (0.9 + 0.6) / 2.0).abs() < 1e-6, "score = 采纳帧均值: {score}");
    }

    /// 间隔 blank 的相同字符保留：A blank A → "AA"。
    #[test]
    fn blank_separated_repeats_are_kept() {
        let labels = vec!["".to_string(), "A".to_string()];
        let mut classes = Vec::new();
        for row in [[0.0, 1.0], [1.0, 0.0], [0.0, 1.0]] {
            classes.extend_from_slice(&row);
        }
        let (text, _) = ctc_greedy_decode(&classes, 3, 2, &labels);
        assert_eq!(text, "AA");
    }

    /// 无 blank 间隔的连续同字符折叠：A A A → "A"。
    #[test]
    fn consecutive_same_char_collapses() {
        let labels = vec!["".to_string(), "汉".to_string()];
        let mut classes = Vec::new();
        for row in [[0.0, 1.0], [0.0, 1.0], [0.0, 1.0]] {
            classes.extend_from_slice(&row);
        }
        let (text, _) = ctc_greedy_decode(&classes, 3, 2, &labels);
        assert_eq!(text, "汉");
    }

    /// 标签表：blank + 字典 + （类别数恰好多 1 时）末位空格。
    #[test]
    fn labels_include_trailing_space_for_v4_rec() {
        let dict = vec!["一".to_string(), "二".to_string()];
        // v4 rec：6625 = 1 + 6623 + space → 此处 4 = 1 + 2 + 1。
        let labels = build_labels(dict.clone(), 4);
        assert_eq!(labels[0], "");
        assert_eq!(labels[1], "一");
        assert_eq!(labels[2], "二");
        assert_eq!(labels[3], " ");
        // 类别数不吻合（无空格位）时不加空格。
        let labels2 = build_labels(dict, 3);
        assert_eq!(labels2.len(), 3);
        assert_eq!(labels2[2], "二");
    }

    /// 全 blank 序列 → 空文本 + 0 分。
    #[test]
    fn all_blank_yields_empty() {
        let labels = vec!["".to_string(), "A".to_string()];
        let classes = [1.0f32, 0.0, 1.0, 0.0];
        let (text, score) = ctc_greedy_decode(&classes, 2, 2, &labels);
        assert_eq!((text.as_str(), score), ("", 0.0));
    }

    /// 尺寸不符防御。
    #[test]
    fn malformed_decode_is_safe() {
        let labels = vec!["".to_string(), "A".to_string()];
        assert_eq!(ctc_greedy_decode(&[], 2, 2, &labels).0, "");
        assert_eq!(ctc_greedy_decode(&[0.0; 3], 2, 2, &labels).0, "");
    }
}
