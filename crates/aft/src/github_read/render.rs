use std::cmp::Ordering;

use super::bot_compress::{compress_discussion_body, compress_document_body, structural_strip};
use super::fetch::GithubReadError;
use super::model::{
    GithubComment, GithubDocument, GithubDocumentKind, GithubReaction, GithubReview,
    GithubReviewCommentSection,
};
use super::resource::{GithubCommentSelector, GithubResource};

/// The maximum number of newest comments rendered for an issue or one pull
/// request review-comment section. Older comments remain available on GitHub
/// and are disclosed in the canonical document instead of silently dropped.
pub const MAX_RENDERED_COMMENTS_PER_SECTION: usize = 50;

/// Render the complete, transport-independent GitHub document once in Rust.
///
/// Selector handling belongs after this function. The renderer purposefully
/// never sees a vision capability or an attachment downloader, so its bytes are
/// identical for text-only and vision-capable callers.
pub fn render_document(document: &GithubDocument) -> String {
    let resource = GithubResource {
        kind: document.resource_kind(),
        number: document.number,
        repository: Some(document.repository.clone()),
        comment_selector: None,
    };
    render_document_for_resource(document, &resource)
        .expect("a whole-document render has no fallible selector")
}

pub(super) fn render_document_for_resource(
    document: &GithubDocument,
    resource: &GithubResource,
) -> Result<String, GithubReadError> {
    if let Some(selector) = &resource.comment_selector {
        return render_selected_discussion(document, selector);
    }

    let mut output = String::new();
    let resource_label = match document.kind {
        GithubDocumentKind::Issue => "Issue",
        GithubDocumentKind::PullRequest => "Pull request",
    };
    output.push_str(&format!(
        "# {resource_label} #{}: {}\n\n",
        document.number, document.title
    ));
    output.push_str(&format!("Repository: {}\n", document.repository));
    output.push_str(&format!("State: {}\n", document.state));
    if let Some(author) = nonempty(&document.author) {
        output.push_str(&format!("Author: {}\n", format_author(author)));
    }
    if let Some(created_at) = nonempty(&document.created_at) {
        output.push_str(&format!("Created: {created_at}\n"));
    }
    if let Some(updated_at) = nonempty(&document.updated_at) {
        output.push_str(&format!("Updated: {updated_at}\n"));
    }
    if !document.labels.is_empty() {
        output.push_str(&format!("Labels: {}\n", document.labels.join(", ")));
    }
    if !document.assignees.is_empty() {
        output.push_str(&format!(
            "Assignees: {}\n",
            document
                .assignees
                .iter()
                .map(|assignee| format_author(assignee))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(milestone) = nonempty(&document.milestone) {
        output.push_str(&format!("Milestone: {milestone}\n"));
    }
    if let Some(reactions) = render_reactions(&document.reactions) {
        output.push_str(&format!("Reactions: {reactions}\n"));
    }

    output.push_str("\n## Body\n\n");
    append_body(&mut output, &compress_document_body(&document.body).body);

    let mut next_ordinal = 1;
    render_comments(
        &mut output,
        "Comments",
        &document.comments,
        document.comments_total_count,
        document.minimized_comments_count,
        &mut next_ordinal,
        resource,
    );

    if document.kind == GithubDocumentKind::PullRequest {
        render_files(&mut output, document);
        render_reviews(&mut output, document, &mut next_ordinal, resource);
        render_review_comment_sections(
            &mut output,
            &document.review_comment_sections,
            &mut next_ordinal,
            resource,
        );
    }

    output.push_str(&format!(
        "Discussion drill-down: {}/comments/<sel> (for example 3, 3-5, or 3,7).\n\n",
        resource.base_spelling()
    ));
    Ok(output)
}

fn render_comments(
    output: &mut String,
    heading: &str,
    comments: &[GithubComment],
    total_count: Option<usize>,
    supplied_minimized_count: Option<usize>,
    next_ordinal: &mut usize,
    resource: &GithubResource,
) {
    if comments.is_empty()
        && total_count.unwrap_or(0) == 0
        && supplied_minimized_count.unwrap_or(0) == 0
    {
        return;
    }
    output.push_str(&format!("\n## {heading}\n\n"));
    let displayed = newest_comments(comments);
    let total_count = total_count.unwrap_or(comments.len()).max(comments.len());
    let omitted = total_count.saturating_sub(displayed.len());
    if omitted > 0 {
        output.push_str(&format!("{omitted} earlier comments omitted\n\n"));
    }
    for comment in displayed {
        render_default_discussion_item(
            output,
            comment.author.as_deref(),
            comment.created_at.as_deref(),
            None,
            &comment.body,
            next_ordinal,
            resource,
        );
    }
    let observed_minimized = comments.iter().filter(|comment| comment.minimized).count();
    let minimized = supplied_minimized_count
        .unwrap_or(observed_minimized)
        .max(observed_minimized);
    if minimized > 0 {
        output.push_str(&format!("Minimized comments: {minimized}\n\n"));
    }
}

fn render_files(output: &mut String, document: &GithubDocument) {
    if document.files.is_empty() {
        return;
    }
    output.push_str("\n## Files\n\n");
    for file in &document.files {
        output.push_str("- `");
        output.push_str(&file.path);
        output.push('`');
        if file.additions.is_some() || file.deletions.is_some() {
            output.push_str(&format!(
                " (+{} -{})",
                file.additions.unwrap_or(0),
                file.deletions.unwrap_or(0)
            ));
        }
        if let Some(status) = nonempty(&file.status) {
            output.push_str(&format!(" [{status}]"));
        }
        output.push('\n');
    }
}

fn render_reviews(
    output: &mut String,
    document: &GithubDocument,
    next_ordinal: &mut usize,
    resource: &GithubResource,
) {
    if !document.reviews.iter().any(review_is_visible) {
        return;
    }
    output.push_str("\n## Reviews\n\n");
    for review in &document.reviews {
        render_review(output, review, next_ordinal, resource);
    }
}

fn render_review(
    output: &mut String,
    review: &GithubReview,
    next_ordinal: &mut usize,
    resource: &GithubResource,
) {
    render_default_discussion_item(
        output,
        review.author.as_deref(),
        review.submitted_at.as_deref(),
        review.state.as_deref(),
        &review.body,
        next_ordinal,
        resource,
    );
}

fn render_review_comment_sections(
    output: &mut String,
    sections: &[GithubReviewCommentSection],
    next_ordinal: &mut usize,
    resource: &GithubResource,
) {
    if !sections.iter().any(|section| {
        section.comments.iter().any(comment_is_visible)
            || section.comments_total_count.unwrap_or(0) > section.comments.len()
            || section.minimized_comments_count.unwrap_or(0) > 0
    }) {
        return;
    }
    output.push_str("\n## Review comments\n\n");
    for section in sections {
        let displayed = newest_comments(&section.comments);
        let total_count = section
            .comments_total_count
            .unwrap_or(section.comments.len())
            .max(section.comments.len());
        let omitted = total_count.saturating_sub(displayed.len());
        if omitted > 0 {
            let author = section
                .author
                .as_deref()
                .map(format_author)
                .unwrap_or_else(|| "unknown".to_string());
            output.push_str(&format!(
                "{omitted} earlier comments omitted from {author}'s review\n\n"
            ));
        }
        for comment in displayed {
            render_default_discussion_item(
                output,
                comment.author.as_deref(),
                comment.created_at.as_deref(),
                None,
                &comment.body,
                next_ordinal,
                resource,
            );
        }
        let observed_minimized = section
            .comments
            .iter()
            .filter(|comment| comment.minimized)
            .count();
        let minimized = section
            .minimized_comments_count
            .unwrap_or(observed_minimized)
            .max(observed_minimized);
        if minimized > 0 {
            output.push_str(&format!("Minimized comments: {minimized}\n\n"));
        }
    }
}

fn render_default_discussion_item(
    output: &mut String,
    author: Option<&str>,
    date: Option<&str>,
    state: Option<&str>,
    body: &str,
    next_ordinal: &mut usize,
    resource: &GithubResource,
) {
    let compressed = compress_discussion_body(author, body);
    if compressed.body.is_empty() {
        return;
    }
    let ordinal = *next_ordinal;
    *next_ordinal += 1;
    render_item_heading(output, ordinal, author, date);
    if let Some(state) = state.filter(|state| !state.is_empty()) {
        output.push_str(&format!("State: {state}\n\n"));
    }
    append_body(output, &compressed.body);
    if compressed.compressed {
        output.push_str(&format!(
            "[compressed; full: {}/comments/{ordinal}]\n\n",
            resource.base_spelling()
        ));
    }
}

fn render_selected_discussion(
    document: &GithubDocument,
    selector: &GithubCommentSelector,
) -> Result<String, GithubReadError> {
    let items = discussion_items(document);
    if let Some(ordinal) = selector.first_out_of_range(items.len()) {
        let valid_range = if items.is_empty() {
            "empty".to_string()
        } else {
            format!("1-{}", items.len())
        };
        return Err(GithubReadError::InvalidCommentSelector(format!(
            "discussion ordinal {ordinal} is out of range; valid range is {valid_range}"
        )));
    }

    let mut output = String::new();
    for (index, item) in items.into_iter().enumerate() {
        let ordinal = index + 1;
        if !selector.contains(ordinal) {
            continue;
        }
        render_item_heading(&mut output, ordinal, item.author, item.date);
        if let Some(state) = item.state.filter(|state| !state.is_empty()) {
            output.push_str(&format!("State: {state}\n\n"));
        }
        append_body(&mut output, &structural_strip(item.body));
    }
    Ok(output)
}

#[derive(Clone, Copy)]
struct DiscussionItem<'a> {
    author: Option<&'a str>,
    date: Option<&'a str>,
    state: Option<&'a str>,
    body: &'a str,
}

fn discussion_items(document: &GithubDocument) -> Vec<DiscussionItem<'_>> {
    let mut items = Vec::new();
    for comment in newest_comments(&document.comments) {
        if comment_is_visible(comment) {
            items.push(DiscussionItem {
                author: comment.author.as_deref(),
                date: comment.created_at.as_deref(),
                state: None,
                body: &comment.body,
            });
        }
    }
    if document.kind == GithubDocumentKind::PullRequest {
        for review in &document.reviews {
            if review_is_visible(review) {
                items.push(DiscussionItem {
                    author: review.author.as_deref(),
                    date: review.submitted_at.as_deref(),
                    state: review.state.as_deref(),
                    body: &review.body,
                });
            }
        }
        for section in &document.review_comment_sections {
            for comment in newest_comments(&section.comments) {
                if comment_is_visible(comment) {
                    items.push(DiscussionItem {
                        author: comment.author.as_deref(),
                        date: comment.created_at.as_deref(),
                        state: None,
                        body: &comment.body,
                    });
                }
            }
        }
    }
    items
}

fn review_is_visible(review: &GithubReview) -> bool {
    discussion_body_is_visible(review.author.as_deref(), &review.body)
}

fn comment_is_visible(comment: &GithubComment) -> bool {
    discussion_body_is_visible(comment.author.as_deref(), &comment.body)
}

fn discussion_body_is_visible(author: Option<&str>, body: &str) -> bool {
    !compress_discussion_body(author, body).body.is_empty()
}

fn render_item_heading(
    output: &mut String,
    ordinal: usize,
    author: Option<&str>,
    date: Option<&str>,
) {
    let author = author
        .map(format_author)
        .unwrap_or_else(|| "unknown".to_string());
    let date = date.unwrap_or("unknown date");
    output.push_str(&format!("### [{ordinal}] {author} · {date}\n\n"));
}

fn newest_comments(comments: &[GithubComment]) -> Vec<&GithubComment> {
    let mut comments: Vec<_> = comments.iter().collect();
    comments.sort_by(|left, right| compare_timestamp(&left.created_at, &right.created_at));
    let drop_count = comments
        .len()
        .saturating_sub(MAX_RENDERED_COMMENTS_PER_SECTION);
    comments.into_iter().skip(drop_count).collect()
}

fn compare_timestamp(left: &Option<String>, right: &Option<String>) -> Ordering {
    left.as_deref()
        .unwrap_or("")
        .cmp(right.as_deref().unwrap_or(""))
}

fn render_reactions(reactions: &[GithubReaction]) -> Option<String> {
    let rendered: Vec<_> = reactions
        .iter()
        .filter(|reaction| reaction.count > 0)
        .map(|reaction| {
            format!(
                "{} x{}",
                reaction_display(&reaction.content),
                reaction.count
            )
        })
        .collect();
    (!rendered.is_empty()).then(|| rendered.join(", "))
}

fn reaction_display(reaction: &str) -> &str {
    match reaction {
        "THUMBS_UP" | "+1" => "+1",
        "THUMBS_DOWN" | "-1" => "-1",
        "LAUGH" => "laugh",
        "HOORAY" => "hooray",
        "CONFUSED" => "confused",
        "HEART" => "heart",
        "ROCKET" => "rocket",
        "EYES" => "eyes",
        other => other,
    }
}

fn format_author(author: &str) -> String {
    if author.starts_with('@') {
        author.to_string()
    } else {
        format!("@{author}")
    }
}

fn nonempty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

fn append_body(output: &mut String, body: &str) {
    if body.is_empty() {
        return;
    }
    output.push_str(body.trim_end());
    output.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_read::model::{GithubDocument, GithubDocumentKind};

    #[test]
    fn renderer_keeps_full_human_body_caps_comments_and_omits_comment_reactions() {
        let mut document = GithubDocument {
            repository: "owner/repo".to_string(),
            kind: GithubDocumentKind::Issue,
            number: 7,
            title: "Fixture".to_string(),
            state: "OPEN".to_string(),
            author: Some("octo".to_string()),
            body: "body with https://user-images.githubusercontent.com/example.png".to_string(),
            reactions: vec![GithubReaction {
                content: "THUMBS_UP".to_string(),
                count: 2,
            }],
            ..GithubDocument::default()
        };
        document.comments = (0..51)
            .map(|number| GithubComment {
                author: Some("commenter".to_string()),
                body: format!("comment {number}"),
                created_at: Some(format!("2026-01-{:02}T00:00:00Z", number.min(28))),
                minimized: number == 0,
                ..GithubComment::default()
            })
            .collect();
        document.comments_total_count = Some(51);

        let text = render_document(&document);
        assert!(text.contains("Repository: owner/repo"));
        assert!(text.contains("Reactions: +1 x2"));
        assert!(text.contains("1 earlier comments omitted"));
        assert!(text.contains("Minimized comments: 1"));
        assert!(!text.contains("comment 0\n"));
        assert!(text.contains("### [1] @commenter"));
        assert!(text.contains("### [50] @commenter"));
        assert_eq!(text.matches("/comments/<sel>").count(), 1);
    }
}
