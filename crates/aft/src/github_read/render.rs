use std::cmp::Ordering;

use super::model::{
    GithubComment, GithubDocument, GithubDocumentKind, GithubReaction, GithubReview,
    GithubReviewCommentSection,
};

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
    append_body(&mut output, &document.body);

    render_comments(
        &mut output,
        "Comments",
        &document.comments,
        document.comments_total_count,
        document.minimized_comments_count,
    );

    if document.kind == GithubDocumentKind::PullRequest {
        render_files(&mut output, document);
        render_reviews(&mut output, document);
        render_review_comment_sections(&mut output, &document.review_comment_sections);
    }

    output
}

fn render_comments(
    output: &mut String,
    heading: &str,
    comments: &[GithubComment],
    total_count: Option<usize>,
    supplied_minimized_count: Option<usize>,
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
        render_comment(output, comment, "###");
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

fn render_reviews(output: &mut String, document: &GithubDocument) {
    if document.reviews.is_empty() {
        return;
    }
    output.push_str("\n## Reviews\n\n");
    for review in &document.reviews {
        render_review(output, review);
    }
}

fn render_review(output: &mut String, review: &GithubReview) {
    let author = review
        .author
        .as_deref()
        .map(format_author)
        .unwrap_or_else(|| "unknown".to_string());
    let date = review.submitted_at.as_deref().unwrap_or("unknown date");
    output.push_str(&format!("### {author} · {date}\n\n"));
    if let Some(state) = nonempty(&review.state) {
        output.push_str(&format!("State: {state}\n\n"));
    }
    append_body(output, &review.body);
}

fn render_review_comment_sections(output: &mut String, sections: &[GithubReviewCommentSection]) {
    if sections.is_empty() {
        return;
    }
    output.push_str("\n## Review comments\n\n");
    for section in sections {
        let author = section
            .author
            .as_deref()
            .map(format_author)
            .unwrap_or_else(|| "unknown".to_string());
        let date = section.submitted_at.as_deref().unwrap_or("unknown date");
        output.push_str(&format!("### {author} · {date}\n\n"));
        let displayed = newest_comments(&section.comments);
        let total_count = section
            .comments_total_count
            .unwrap_or(section.comments.len())
            .max(section.comments.len());
        let omitted = total_count.saturating_sub(displayed.len());
        if omitted > 0 {
            output.push_str(&format!("{omitted} earlier comments omitted\n\n"));
        }
        for comment in displayed {
            render_comment(output, comment, "####");
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

fn render_comment(output: &mut String, comment: &GithubComment, marker: &str) {
    let author = comment
        .author
        .as_deref()
        .map(format_author)
        .unwrap_or_else(|| "unknown".to_string());
    let date = comment.created_at.as_deref().unwrap_or("unknown date");
    output.push_str(&format!("{marker} {author} · {date}\n\n"));
    append_body(output, &comment.body);
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
        output.push_str("(none)\n\n");
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
    fn renderer_keeps_full_body_caps_comments_and_omits_comment_reactions() {
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
    }
}
