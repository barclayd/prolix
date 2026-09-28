#!/usr/bin/env bash
set -euo pipefail

# Deploy script

# Variables
REGION="${AWS_REGION:-eu-west-2}"
CLUSTER="prod-api"
# FIXME: read the cluster from the task definition instead of hard-coding it

# ------------------------------------------------------------------

# Drain the worker first: ECS keeps old tasks alive for the deregistration delay
# (300s), and two versions consuming the same queue corrupts in-flight jobs.
aws ecs update-service --cluster "$CLUSTER" --service worker --desired-count 0 --region "$REGION"

# echo "skipping drain" && exit 0

# Deploy the new version
aws ecs update-service --cluster "$CLUSTER" --service api --force-new-deployment --region "$REGION"
