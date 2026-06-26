    use super::*;

    #[test]
    fn prometheus_text_has_type_and_help_lines() {
        let s = render_prometheus_text();
        assert!(s.contains("# HELP den_chat_send_started_total"));
        assert!(s.contains("# TYPE den_chat_send_started_total counter"));
        assert!(s.contains("den_chat_send_started_total "));
        assert!(s.contains("# HELP den_chat_send_runtime_legacy_total"));
        assert!(s.contains("# HELP den_chat_send_runtime_bear_channel_total"));
        assert!(s.contains("# HELP den_chat_send_ttfb_ms_sum"));
        assert!(s.contains("# HELP den_chat_send_dropped_total"));
    }

    #[test]
    fn ttfb_and_drop_counters_increment() {
        record_chat_send_ttfb_ms(42);
        record_chat_send_dropped(false);
        record_chat_send_dropped(true);
        let s = render_prometheus_text();
        assert!(s.contains("den_chat_send_ttfb_ms_sum 42"));
        assert!(s.contains("den_chat_send_ttfb_ms_count 1"));
        assert!(s.contains("den_chat_send_dropped_total 2"));
        assert!(s.contains("den_chat_send_dropped_with_bytes_total 1"));
    }
