use tracing::info;
use ts_rs::ExportError;

pub struct TypeExport {
    pub name: &'static str,
    pub export_all: fn() -> Result<(), ExportError>,
}

inventory::collect!(TypeExport);

pub fn export_all_types() -> anyhow::Result<()> {
    for export in inventory::iter::<TypeExport> {
        (export.export_all)().map_err(|e| anyhow::anyhow!("Failed to export {}: {}", export.name, e))?;
        info!("Exported {}", export.name);
    }

    Ok(())
}
