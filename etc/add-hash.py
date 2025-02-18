#! /usr/bin/env python

import os
import subprocess

input = "./docs/top-index.html"
output = "rustdocs/index.html"

# Get the commit from github (otherwise, we are likely to just see a merge commit.  Default to HEAD
# if not provided.)
commit_sha = os.getenv("COMMIT_SHA", "HEAD")

try:
    commit_info = subprocess.check_output(["git", "show", "--stat", commit_sha], text=True)
except subprocess.CalledProcessError as e:
    print(f"Error getting commit info: {e}")
    commit_info = "Error retrieving commit details.\n"

with open(output, "w") as outfile:
    with open(input, "r") as file:
        for line in file:
            if "{{COMMIT}}" in line:
                outfile.write(commit_info)
            else:
                outfile.write(line)
