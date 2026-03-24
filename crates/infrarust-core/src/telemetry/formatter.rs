//! Custom tracing formatter with colored output, emoji level icons, and TTY detection.

use std::fmt;
use std::io::IsTerminal;

use tracing::Level;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

pub struct InfrarustFormatter {
    is_tty: bool,
}

impl InfrarustFormatter {
    pub fn new() -> Self {
        Self {
            is_tty: std::io::stdout().is_terminal(),
        }
    }
}

impl Default for InfrarustFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, N> FormatEvent<S, N> for InfrarustFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        let level = *event.metadata().level();

        let now = std::time::SystemTime::now();
        let datetime = humantime::format_rfc3339_seconds(now);
        if self.is_tty {
            write!(writer, "{} ", console::style(datetime).dim())?;
        } else {
            write!(writer, "{datetime} ")?;
        }

        write_level(&mut writer, level, self.is_tty)?;
        write!(writer, " ")?;

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if let Some(ref msg) = visitor.message {
            if self.is_tty {
                match level {
                    Level::ERROR => write!(writer, "{}", console::style(msg).red().bold())?,
                    Level::WARN => write!(writer, "{}", console::style(msg).yellow().bold())?,
                    _ => write!(writer, "{msg}")?,
                }
            } else {
                write!(writer, "{msg}")?;
            }
        }

        if !visitor.fields.is_empty() {
            write!(writer, " ")?;
            if self.is_tty {
                write!(writer, "{}", console::style("{").dim())?;
                for (i, (key, value)) in visitor.fields.iter().enumerate() {
                    if i > 0 {
                        write!(writer, "{}", console::style(", ").dim())?;
                    }
                    write!(
                        writer,
                        "{}{}{}",
                        console::style(key).dim(),
                        console::style("=").dim(),
                        console::style(value).cyan(),
                    )?;
                }
                write!(writer, "{}", console::style("}").dim())?;
            } else {
                write!(writer, "{{")?;
                for (i, (key, value)) in visitor.fields.iter().enumerate() {
                    if i > 0 {
                        write!(writer, ", ")?;
                    }
                    write!(writer, "{key}={value}")?;
                }
                write!(writer, "}}")?;
            }
        }

        if let Some(scope) = ctx.event_scope() {
            let mut has_spans = false;
            for span in scope.from_root() {
                if !has_spans {
                    write!(writer, " ")?;
                    has_spans = true;
                } else if self.is_tty {
                    write!(writer, "{}", console::style(" → ").blue())?;
                } else {
                    write!(writer, " > ")?;
                }

                let name = span.metadata().name();
                if self.is_tty {
                    write!(writer, "{}", console::style(name).blue())?;
                } else {
                    write!(writer, "{name}")?;
                }

                let ext = span.extensions();
                if let Some(fields) =
                    ext.get::<tracing_subscriber::fmt::FormattedFields<N>>()
                {
                    if !fields.is_empty() {
                        if self.is_tty {
                            write!(writer, "{}", console::style(format!(":{fields}")).blue())?;
                        } else {
                            write!(writer, ":{fields}")?;
                        }
                    }
                }
            }
        }

        writeln!(writer)
    }
}

fn write_level(writer: &mut Writer<'_>, level: Level, is_tty: bool) -> fmt::Result {
    if is_tty {
        match level {
            Level::TRACE => write!(writer, "{}", console::style("🔍 TRACE").white()),
            Level::DEBUG => write!(writer, "{}", console::style("🔧 DEBUG").cyan()),
            Level::INFO => write!(writer, "{}", console::style("✅ INFO ").green()),
            Level::WARN => write!(writer, "{}", console::style("⚠️  WARN ").yellow()),
            Level::ERROR => write!(writer, "{}", console::style("❌ ERROR").red()),
        }
    } else {
        match level {
            Level::TRACE => write!(writer, "TRACE"),
            Level::DEBUG => write!(writer, "DEBUG"),
            Level::INFO => write!(writer, "INFO "),
            Level::WARN => write!(writer, "WARN "),
            Level::ERROR => write!(writer, "ERROR"),
        }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let val = format!("{value:?}");
        let val = val.strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(&val)
            .to_string();

        if field.name() == "message" {
            self.message = Some(val);
        } else {
            self.fields.push((field.name().to_string(), val));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
}

#[allow(clippy::print_stdout)]
pub fn print_banner() {
    let banner = r"
                                                                      
                                                                      
                              .........                               
                           ................                           
                       .......................                        
                   ...............................                    
                ......................................                
            .............................................             
         ....................................................         
        ......................................................        
        .....................................................         
      -----............................................... +++++      
      -------- ....................................... +++++++++      
      ------------................................. ++++++++++++      
      --------------- .........................  +++++++++++++++      
      ------------------  .................. +++++++++++++++++++      
      ---------------------- ...........  ++++++++++++++++++++++      
      -------------------------  ...  ++++++++++++++++++++++++++      
      ----------------------------  ++++++++++++++++++++++++++++      
      ----------------------------  ++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
      ---------------------------- +++++++++++++++++++++++++++++      
        -------------------------- +++++++++++++++++++++++++++        
           ----------------------- +++++++++++++++++++++++            
               ------------------- ++++++++++++++++++++               
                  ---------------- ++++++++++++++++                   
                      ------------ +++++++++++++                      
                         --------- +++++++++                          
                             ----- ++++++                             
                                --  +                                                              
";
    let version = format!("Infrarust - v{}", env!("CARGO_PKG_VERSION"));

    if std::io::stdout().is_terminal() {
        println!("{}", console::style(banner).yellow());
        println!("{}", console::style(version).dim());
    } else {
        println!("{banner}");
        println!("{version}");
    }
    println!();
}
