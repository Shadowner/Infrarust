use tracing::Level;
use tracing_subscriber::fmt::{FormatEvent, FormatFields};
use std::fmt;
use std::collections::HashMap;

pub struct InfrarustMessageFormatter {
    show_target: bool,
    show_level: bool,
    time_format: String,
    timestamp: bool,
    all_fields: bool,
    use_icons: bool,
    print_template: String,
    field_prefixes: HashMap<String, String>,
    use_ansi: bool,
}

impl Default for InfrarustMessageFormatter {
    fn default() -> Self {
        Self {
            show_target: true,
            show_level: true,
            time_format: "%Y-%m-%d %H:%M:%S%.3f".to_string(),
            timestamp: true,
            all_fields: true,
            use_icons: true,
            print_template: "{timestamp} {level} {target} {message} {fields}".to_string(),
            field_prefixes: HashMap::new(),
            use_ansi: true,
        }
    }
}

impl InfrarustMessageFormatter {
    pub fn with_icons(mut self, use_icons: bool) -> Self {
        self.use_icons = use_icons;
        self
    }

    pub fn with_timestamp(mut self, enabled: bool) -> Self {
        self.timestamp = enabled;
        self
    }

    pub fn with_target(mut self, show_target: bool) -> Self {
        self.show_target = show_target;
        self
    }

    pub fn with_level(mut self, show_level: bool) -> Self {
        self.show_level = show_level;
        self
    }

    pub fn with_all_fields(mut self, all_fields: bool) -> Self {
        self.all_fields = all_fields;
        self
    }

    pub fn with_time_format(mut self, format: &str) -> Self {
        self.time_format = format.to_string();
        self
    }
    
    pub fn with_template(mut self, template: &str) -> Self {
        self.print_template = template.to_string();
        self
    }
    
    pub fn before_field(mut self, field_name: &str, prefix: &str) -> Self {
        self.field_prefixes.insert(field_name.to_string(), prefix.to_string());
        self
    }

    pub fn with_ansi(mut self, enabled: bool) -> Self {
        self.use_ansi = enabled;
        self
    }
    
    // Helper method to conditionally apply ANSI color codes
    fn colorize(&self, text: &str, color_code: &str) -> String {
        if self.use_ansi {
            format!("{}{}\x1B[0m", color_code, text)
        } else {
            text.to_string()
        }
    }
    
    // Helper to replace template placeholders with values
    fn apply_template(&self, template: &str, values: &HashMap<&str, String>) -> String {
        let mut result = template.to_string();
        for (key, value) in values {
            // Only include non-empty field values and their potential prefixes
            if !value.is_empty() {
                let placeholder = format!("{{{}}}", key);
                
                // Check if we have a prefix for this field
                let replacement = if let Some(prefix) = self.field_prefixes.get(*key) {
                    format!("{}{}", prefix, value)
                } else {
                    value.clone()
                };
                
                result = result.replace(&placeholder, &replacement);
            } else {
                // If the field is empty, remove its placeholder completely
                result = result.replace(&format!("{{{}}}", key), "");
            }
        }
        result
    }
}

impl<S, N> FormatEvent<S, N> for InfrarustMessageFormatter
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        // Collect all components that will be used in the template
        let mut components: HashMap<&str, String> = HashMap::new();
        
        // Format timestamp
        if self.timestamp {
            let now = chrono::Local::now();
            let timestamp = now.format(&self.time_format);
            components.insert("timestamp", self.colorize(&timestamp.to_string(), "\x1B[38;5;240m"));
        } else {
            components.insert("timestamp", String::new());
        }
        
        // Format log level with icon
        let level = event.metadata().level();
        if self.show_level {
            if self.use_icons {
                match *level {
                    Level::TRACE => components.insert("level", self.colorize("🔍  TRACE", "\x1B[37m")),
                    Level::DEBUG => components.insert("level", self.colorize("🔧  DEBUG", "\x1B[36m")),
                    Level::INFO  => components.insert("level", self.colorize("✅  INFO ", "\x1B[32m")),
                    Level::WARN  => components.insert("level", self.colorize("⚠️  WARN ", "\x1B[33m")),
                    Level::ERROR => components.insert("level", self.colorize("❌  ERROR", "\x1B[31m")),
                };
            } else {
                match *level {
                    Level::TRACE => components.insert("level", self.colorize("TRACE", "\x1B[37m")),
                    Level::DEBUG => components.insert("level", self.colorize("DEBUG", "\x1B[36m")),
                    Level::INFO  => components.insert("level", self.colorize("INFO ", "\x1B[32m")),
                    Level::WARN  => components.insert("level", self.colorize("WARN", "\x1B[33m")),
                    Level::ERROR => components.insert("level", self.colorize("ERROR", "\x1B[31m")),
                };
            }
        } else {
            components.insert("level", String::new());
        }
        
        // Format target/context
        if self.show_target {
            let target = event.metadata().target();
            components.insert("target", self.colorize(target, "\x1B[35m"));
        } else {
            components.insert("target", String::new());
        }
        
        // Format spans if any
        let mut spans = String::new();
        if let Some(scope) = ctx.event_scope() {
            let mut seen = false;
            for span in scope.from_root() {
                if seen {
                    spans.push_str("→");
                } else if self.use_ansi {
                    spans.push_str("\x1B[34m");
                }
                seen = true;
                spans.push_str(&format!("{}:", span.metadata().name()));
                
                let ext = span.extensions();
                if let Some(fields) = ext.get::<tracing_subscriber::fmt::FormattedFields<N>>() {
                    spans.push_str(&format!("{}", fields));
                }
            }
            if seen && self.use_ansi {
                spans.push_str("\x1B[0m ");
            }
        }
        components.insert("spans", spans);
        
        // Extract message and fields
        struct MessageVisitor {
            message: Option<String>,
            fields: Vec<(String, String)>,
        }
        
        impl tracing::field::Visit for MessageVisitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
                let val_str = format!("{:?}", value).trim_matches('"').to_string();
                
                if field.name() == "message" {
                    self.message = Some(val_str);
                } else {
                    self.fields.push((field.name().to_string(), val_str));
                }
            }

            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "message" {
                    self.message = Some(value.to_string());
                } else {
                    self.fields.push((field.name().to_string(), value.to_string()));
                }
            }
            
            fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
                self.fields.push((field.name().to_string(), value.to_string()));
            }
            
            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                self.fields.push((field.name().to_string(), value.to_string()));
            }
            
            fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
                self.fields.push((field.name().to_string(), value.to_string()));
            }
            
            fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
                self.fields.push((field.name().to_string(), value.to_string()));
            }
        }
        
        let mut visitor = MessageVisitor {
            message: None,
            fields: Vec::new(),
        };
        
        event.record(&mut visitor);
        
        // Format message with styling based on log level
        let message = if let Some(msg) = visitor.message {
            match *level {
                Level::ERROR => self.colorize(&msg, "\x1B[1;31m"),
                Level::WARN => self.colorize(&msg, "\x1B[1;33m"),
                _ => msg,
            }
        } else {
            String::new()
        };
        components.insert("message", message);
        
        // Format fields
        let fields_str = if self.all_fields && !visitor.fields.is_empty() {
            let mut fields = String::new();
            if self.use_ansi {
                fields.push_str("\x1B[90m{");
            } else {
                fields.push_str("{");
            }
            
            let mut first = true;
            for (key, value) in visitor.fields {
                if !first {
                    fields.push_str(", ");
                }
                first = false;
                
                if self.use_ansi {
                    fields.push_str(&format!("{}=\x1B[36m{}\x1B[90m", key, value));
                } else {
                    fields.push_str(&format!("{}={}", key, value));
                }
            }
            
            if self.use_ansi {
                fields.push_str("}\x1B[0m");
            } else {
                fields.push_str("}");
            }
            
            fields
        } else {
            String::new()
        };
        components.insert("fields", fields_str);
        
        // Apply the template and write the result
        let formatted = self.apply_template(&self.print_template, &components);
        write!(writer, "{}", formatted)?;
        
        writeln!(writer)
    }
}