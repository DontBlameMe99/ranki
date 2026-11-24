# Agent instructions

## Pull requests with code changes

Any pull request that changes code must show the result. When the change is visible, embed two images in the PR description: a **Before** screenshot and an **After** screenshot. Label both images so reviewers know which is which.

CI treats every change to a source or workflow file as a code change and fails the PR when the description has fewer than two embedded images. Documentation-only PRs are exempt.
