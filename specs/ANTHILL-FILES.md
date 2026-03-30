# ANTHILL-FILES: File Management and Workspace Access

| Field      | Value                                                        |
|------------|--------------------------------------------------------------|
| Version    | 0.1 Draft                                                    |
| Date       | 2026-03-30                                                   |
| Status     | Draft                                                        |
| Depends on | ANTHILL-SENTANT, ANTHILL-DASHBOARD                           |
| Related    | ANTHILL-KNOWLEDGE, ANTHILL-REPORTS                            |

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
> "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in RFC 2119.

---

## 1. Introduction

Every ANT operates within a sandboxed working directory that isolates its
files from the host system and from other ANTs. The working directory
contains three well-known subdirectories -- `files/`, `repos/`, and
`memory/` -- each serving a distinct purpose. The file management subsystem
provides a REST API for uploading, downloading, listing, and deleting files,
and a browser-based UI for navigating the workspace.

The file system serves two audiences: human operators who inspect or supply
reference material, and the ANT's own AI workers that read and write files
during rumination (e.g. caching fetched citation content in `files/`).

---

## 2. Directory Structure

When an ANT starts, the runtime MUST create the following directory tree
beneath the ANT's `working_dir`:

```
<working_dir>/
  files/          — User-uploaded files and ANT-cached content
  repos/          — Cloned repositories (excluded from Git backup)
  memory/         — Knowledge graphs, topic graphs, rumination logs
    graphs/       — Per-topic graph files (JSON/CBOR)
    questions.json
    rumination_log.json
```

### 2.1 Subdirectory Roles

| Directory  | Purpose                                                                             | Backed up by Git |
|------------|-------------------------------------------------------------------------------------|------------------|
| `files/`   | User-uploaded reference material and citation content cached during rumination.      | Yes              |
| `repos/`   | Cloned Git repositories for code-aware ANTs. Configurable via `repos_dir` in config.| No               |
| `memory/`  | Knowledge graph persistence. Configurable via `memory_dir` in config.               | Yes              |

### 2.2 Default Paths

If the ANT configuration does not specify a `working_dir`, the runtime
MUST default to:

```
$HOME/.config/anthill/ants/<ant-name>/working
```

The `memory_dir` defaults to `"memory"` and `repos_dir` defaults to
`"repos"`, both relative to `working_dir`.

### 2.3 Git Version Control

The runtime MUST initialise a Git repository in `working_dir` on first
start. Periodic commits capture the state of `memory/` and `files/`. The
`repos/` directory SHOULD be excluded from backup commits.

---

## 3. File Browser

The web dashboard (ANTHILL-DASHBOARD) provides a file browser tab that
allows users to navigate the ANT's working directory.

### 3.1 Navigation

When the user selects the "Files" tab, the browser MUST load the root
listing of the ANT's working directory by calling `GET /api/ants/{id}/files`.
Clicking a directory entry MUST navigate into that directory by calling
`GET /api/ants/{id}/files/{path}`.

### 3.2 Breadcrumb Trail

The browser MUST display a breadcrumb trail showing the path from the
workspace root to the current directory. Each segment of the breadcrumb
MUST be clickable, navigating directly to that directory level. The root
segment is labelled "workspace".

### 3.3 Sort Order

Directory listings MUST be sorted with directories first, then files. Within
each group, entries MUST be sorted alphabetically by name (case-sensitive,
lexicographic). The server performs this sort before returning results.

### 3.4 Hidden Entries

The listing MUST hide `.git` and `.gitignore` entries. All other files and
directories within the working directory are visible.

---

## 4. File Upload

### 4.1 Upload Mechanism

The dashboard provides an "Upload" button that opens the browser's native
file picker (with `multiple` selection enabled). Uploaded files are sent
individually via `POST /api/ants/{id}/upload/{path}`.

When the user uploads from a subdirectory, the upload path MUST be prefixed
with the current directory. When uploading from the root, the default target
directory is `files/`.

### 4.2 Upload Size Limit

The maximum upload size is defined by the constant `MAX_UPLOAD_BYTES`:

```rust
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;  // 50 MiB
```

The server MUST enforce this limit at two levels:

1. An Axum `DefaultBodyLimit` layer on the upload route, set to
   `MAX_UPLOAD_BYTES`.
2. An explicit length check in the handler that returns HTTP 413
   (Payload Too Large) with the message `"Upload exceeds 50 MiB limit"`
   if `body.len() > MAX_UPLOAD_BYTES`.

### 4.3 Path Sanitisation

All uploaded paths pass through `resolve_ant_path()` (see Section 8). The
server MUST create any missing parent directories via `create_dir_all`
before writing the file.

### 4.4 Zip Extraction

If the uploaded file has a `.zip` extension or begins with the ZIP magic
bytes (`0x50 0x4B 0x03 0x04`), the server MUST attempt to extract its
contents into the parent directory of the target path using the system
`unzip` command. If extraction fails, the server MUST fall back to saving
the raw zip file. On success, the response MUST return HTTP 201 with a body
indicating the number of extracted files.

---

## 5. File Download

### 5.1 Content-Type Detection

When serving a file via `GET /api/ants/{id}/files/{path}`, the server MUST
set the `Content-Type` header based on the file extension using the
`guess_content_type()` function. The mapping is:

| Extension(s)                                    | Content-Type               |
|-------------------------------------------------|----------------------------|
| `html`, `htm`                                   | `text/html`                |
| `css`                                           | `text/css`                 |
| `js`                                            | `application/javascript`   |
| `json`                                          | `application/json`         |
| `toml`                                          | `text/plain`               |
| `md`                                            | `text/markdown`            |
| `txt`, `log`                                    | `text/plain`               |
| `rs`, `py`, `sh`, `rb`, `go`, `ts`, `c`, `h`   | `text/plain`               |
| `png`                                           | `image/png`                |
| `jpg`, `jpeg`                                   | `image/jpeg`               |
| `gif`                                           | `image/gif`                |
| `svg`                                           | `image/svg+xml`            |
| `pdf`                                           | `application/pdf`          |
| `zip`                                           | `application/zip`          |
| `tar`                                           | `application/x-tar`        |
| (all others)                                    | `application/octet-stream` |

### 5.2 Inline vs Download

The dashboard client decides how to present the response based on the
content type. Previewable types (see Section 6) are rendered inline.
Non-previewable types are offered as a download link. The user MAY
explicitly download any file via the "Download" button, which triggers a
programmatic `<a download>` click.

---

## 6. File Preview

The file browser provides inline preview for the following file categories:

### 6.1 Images

Files with content type `image/*` or extensions `png`, `jpg`, `jpeg`, `gif`,
`svg`, `webp`, or `bmp` are rendered as `<img>` elements. The image is
loaded from a blob URL and constrained to `max-width: 100%`.

### 6.2 JSON

Files with content type `application/json` or the `.json` extension are
parsed and displayed as syntax-highlighted, pretty-printed `<pre>` blocks
with 2-space indentation. If parsing fails, the content is shown as raw
text.

### 6.3 Markdown

Files with content type `text/markdown` or the `.md` extension are rendered
through the dashboard's `formatMarkdown()` function and displayed in a
styled container.

### 6.4 Text and Source Code

Files with content type `text/*` or `application/javascript` are displayed
as plain `<pre>` blocks with HTML escaping.

### 6.5 PDF

Files with content type `application/pdf` or the `.pdf` extension are
rendered in an `<iframe>` spanning the full preview area (80vh height).

### 6.6 Binary Files

All other files display a message "Binary file" with a direct download
link.

---

## 7. File Deletion

### 7.1 Confirmation

The dashboard MUST display a browser `confirm()` dialog before sending a
delete request. The dialog shows the file name (not the full path).

### 7.2 Delete Behaviour

- Files: the server calls `remove_file()`. On success, returns HTTP 200.
- Empty directories: the server calls `remove_dir()`. On success, returns
  HTTP 200. Non-empty directories cannot be deleted (the OS call will fail).
- Non-existent paths: the server returns HTTP 404.

### 7.3 Path Traversal Prevention

All delete paths pass through `resolve_ant_path()` (see Section 8). The
server MUST NOT delete any path that resolves outside the ANT's canonical
working directory.

### 7.4 UI Refresh

After a successful deletion, the dashboard MUST refresh the current
directory listing by re-calling `loadFiles()` with the current path.

---

## 8. Sandbox Security

All file operations MUST be confined to the ANT's working directory. The
`resolve_ant_path()` function enforces this boundary.

### 8.1 Path Resolution Algorithm

Given an ANT identifier and a subpath, the function:

1. Looks up the ANT's `working_dir` from the bot registry.
2. Canonicalises the base directory using `std::fs::canonicalize()` to
   resolve any symlinks in the working directory path itself.
3. Strips leading slashes from the subpath.
4. Rejects paths containing null bytes (returns `None`).
5. Joins the base and cleaned subpath to form the full path.
6. **For existing paths:** canonicalises the full path and verifies that it
   starts with the canonical base. If it does not, logs a warning
   (`"Path traversal blocked"`) and returns `None`.
7. **For new paths (uploads):** canonicalises the parent directory (if it
   exists) and verifies that it starts with the canonical base. Additionally
   performs a lexical prefix check on the joined path to catch cases where
   the parent does not yet exist.

### 8.2 Symlink Policy

The server MUST NOT follow symlinks that escape the working directory. The
canonicalization step resolves symlinks to their real targets, and the
prefix check rejects any resolved path outside the canonical base. This
means a symlink inside `working_dir` that points outside it will be
blocked.

### 8.3 Null Byte Injection

Paths containing null bytes (`\0`) MUST be rejected immediately, before
any filesystem operation.

### 8.4 Error Responses

When path resolution fails (traversal attempt, missing ANT, null bytes),
the server MUST return HTTP 404. The response MUST NOT disclose the
canonical base path or the reason for rejection to the client.

---

## 9. Citation File Cache

During rumination (ANTHILL-RUMINATION), the citation analysis task fetches
external URLs referenced by the ANT's knowledge graph. When a fetch
succeeds, the ANT's AI worker SHOULD save the retrieved content into the
`files/` directory for future reference. This cache serves two purposes:

1. **Avoiding redundant fetches.** Subsequent rumination cycles check
   `files/` before re-fetching a URL.
2. **Offline verification.** Cached content allows the ANT to re-verify
   citations even if the original source becomes unavailable.

The citation consolidation rumination task instructs the AI worker:
> "VERIFY: For each citation with a URL, FETCH it (check files/ first)."
> "If the fetch succeeds, save the content to files/ for future reference."

Broken citations (404, timeout, no relevant content) MUST be removed from
the citations graph and from any topic graph edges that reference them.
Cached files for broken citations SHOULD be cleaned up.

---

## 10. REST API

All file management endpoints are protected routes requiring a valid
credential in the `X-Credential` header. Endpoints are registered under the
`protected_api` router.

### 10.1 List Root Directory

```
GET /api/ants/{id}/files
```

Returns a JSON array of `FileEntry` objects for the root of the ANT's
working directory.

### 10.2 Get File or List Subdirectory

```
GET /api/ants/{id}/files/{*path}
```

- If `{path}` resolves to a **directory**: returns a JSON array of
  `FileEntry` objects.
- If `{path}` resolves to a **file**: returns the file content with the
  appropriate `Content-Type` header (see Section 5.1).
- Otherwise: returns HTTP 404.

### 10.3 Upload File

```
POST /api/ants/{id}/upload/{*path}
```

The request body is the raw file content (not multipart). The route has a
`DefaultBodyLimit` layer set to `MAX_UPLOAD_BYTES` (50 MiB).

**Responses:**

| Status | Condition                                |
|--------|------------------------------------------|
| 201    | File written or zip extracted            |
| 404    | ANT not found or path traversal blocked  |
| 413    | Body exceeds 50 MiB limit               |
| 500    | Filesystem write error                   |

### 10.4 Delete File

```
DELETE /api/ants/{id}/files/{*path}
```

Deletes a file or empty directory.

**Responses:**

| Status | Condition                                |
|--------|------------------------------------------|
| 200    | File or empty directory deleted           |
| 404    | ANT not found, path not found, or traversal blocked |
| 500    | Filesystem delete error                  |

### 10.5 FileEntry Schema

Directory listing responses return a JSON array where each element has the
following shape:

```json
{
  "name": "string",
  "is_dir": true | false,
  "size": 12345
}
```

| Field    | Type    | Description                                      |
|----------|---------|--------------------------------------------------|
| `name`   | String  | File or directory name (not a full path).         |
| `is_dir` | Boolean | `true` if the entry is a directory.               |
| `size`   | Integer | Size in bytes. Always `0` for directories.        |

---

## 11. Conformance

An implementation claiming conformance to ANTHILL-FILES:

1. MUST create `files/`, `repos/`, and `memory/` subdirectories on ANT
   startup.
2. MUST enforce the `MAX_UPLOAD_BYTES` limit (50 MiB) on all uploads.
3. MUST implement the `resolve_ant_path()` algorithm (Section 8.1) or an
   equivalent that prevents path traversal and symlink escapes.
4. MUST reject paths containing null bytes.
5. MUST NOT disclose internal path information in error responses.
6. MUST sort directory listings with directories first, then files, both
   groups in alphabetical order.
7. MUST hide `.git` and `.gitignore` entries from directory listings.
8. MUST set `Content-Type` headers according to the mapping in Section 5.1.
9. MUST support zip extraction on upload when the file is detected as a ZIP
   archive.
10. SHOULD cache fetched citation content in `files/` during rumination.
