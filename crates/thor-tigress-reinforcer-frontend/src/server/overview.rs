//! What the page shows before any record: the datasets, the header's counts,
//! and the categories the picker offers.

use serde::Serialize;

use crate::{
    datasets::{
        app::App,
        titles,
        workspace::{DatasetInfo, Workspace},
    },
    review::category::SlopCategory,
    error::Outcome,
};

/// What the page's dataset switcher shows.
#[derive(Serialize)]
pub(super) struct Datasets {
    datasets: Vec<DatasetInfo>,
    missing: Vec<String>,
}

pub(super) fn datasets(workspace: &Workspace) -> Outcome<Datasets> {
    Ok(Datasets {
        datasets: workspace.infos()?,
        missing: workspace.missing().to_vec(),
    })
}

/// What the header shows.
#[derive(Serialize)]
pub(super) struct Meta {
    total: usize,
    flagged: usize,
    sources: Vec<SourceInfo>,
    collections: Vec<CollectionInfo>,
    file: String,
}

#[derive(Serialize)]
pub(super) struct SourceInfo {
    name: String,
    title: String,
    count: usize,
}

#[derive(Serialize)]
pub(super) struct CollectionInfo {
    name: String,
    title: String,
    source: String,
    count: usize,
}

pub(super) fn meta(app: &App) -> Outcome<Meta> {
    let sources = app
        .index
        .counts()
        .into_iter()
        .map(|source| SourceInfo {
            title: titles::source_title(&source.name),
            name: source.name,
            count: source.count,
        })
        .collect();
    let collections = app
        .index
        .collections()
        .into_iter()
        .map(|collection| CollectionInfo {
            title: titles::collection_title(&collection.folder),
            name: collection.folder,
            source: collection.source,
            count: collection.count,
        })
        .collect();

    Ok(Meta {
        total: app.index.len(),
        flagged: app.flags()?.len(),
        sources,
        collections,
        file: app.index.path().display().to_string(),
    })
}

/// The categories the picker offers.
#[derive(Serialize)]
pub(super) struct Categories {
    categories: Vec<CategoryInfo>,
}

#[derive(Serialize)]
pub(super) struct CategoryInfo {
    name: SlopCategory,
    description: &'static str,
}

pub(super) fn categories() -> Categories {
    let categories = SlopCategory::ALL
        .into_iter()
        .map(|category| CategoryInfo {
            name: category,
            description: category.description(),
        })
        .collect();

    Categories { categories }
}
