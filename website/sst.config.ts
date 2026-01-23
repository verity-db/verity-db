/// <reference path="./.sst/platform/config.d.ts" />

/**
 * SST v3 Infrastructure Configuration for VerityDB
 *
 * Resources:
 * - ECR repository for container images
 * - App Runner service (auto-scaling, pay-per-use)
 *
 * DNS is managed via Cloudflare (not AWS Route 53).
 *
 * Usage:
 *   npx sst deploy --stage production
 */

export default $config({
  app(input) {
    return {
      name: "veritydb",
      removal: input?.stage === "production" ? "retain" : "remove",
      protect: ["production"].includes(input?.stage),
      home: "aws",
      providers: {
        aws: {
          region: "ap-southeast-2", // Sydney
        },
      },
    };
  },
  async run() {
    const isProduction = $app.stage === "production";

    // ECR Repository for container images
    const repo = new aws.ecr.Repository("SiteRepo", {
      name: `veritydb-site-${$app.stage}`,
      forceDelete: !isProduction,
      imageScanningConfiguration: {
        scanOnPush: true,
      },
      imageTagMutability: "MUTABLE",
    });

    // ECR Lifecycle policy to clean up old images
    new aws.ecr.LifecyclePolicy("SiteRepoLifecycle", {
      repository: repo.name,
      policy: JSON.stringify({
        rules: [
          {
            rulePriority: 1,
            description: "Keep last 10 images",
            selection: {
              tagStatus: "any",
              countType: "imageCountMoreThan",
              countNumber: 10,
            },
            action: {
              type: "expire",
            },
          },
        ],
      }),
    });

    // App Runner IAM Role
    const appRunnerRole = new aws.iam.Role("AppRunnerRole", {
      assumeRolePolicy: JSON.stringify({
        Version: "2012-10-17",
        Statement: [
          {
            Effect: "Allow",
            Principal: {
              Service: "build.apprunner.amazonaws.com",
            },
            Action: "sts:AssumeRole",
          },
        ],
      }),
    });

    // Allow App Runner to pull from ECR
    new aws.iam.RolePolicyAttachment("AppRunnerEcrPolicy", {
      role: appRunnerRole.name,
      policyArn:
        "arn:aws:iam::aws:policy/service-role/AWSAppRunnerServicePolicyForECRAccess",
    });

    // Auto-scaling configuration
    const autoScaling = new aws.apprunner.AutoScalingConfigurationVersion(
      "SiteAutoScaling",
      {
        autoScalingConfigurationName: `veritydb-site-${$app.stage}`,
        minSize: 1, // Sydney region requires minimum 1
        maxSize: 2,
        maxConcurrency: 100,
      }
    );

    // App Runner Service
    const appRunner = new aws.apprunner.Service("Site", {
      serviceName: `veritydb-site-${$app.stage}`,
      sourceConfiguration: {
        authenticationConfiguration: {
          accessRoleArn: appRunnerRole.arn,
        },
        imageRepository: {
          imageIdentifier: $interpolate`${repo.repositoryUrl}:latest`,
          imageRepositoryType: "ECR",
          imageConfiguration: {
            port: "3000",
            runtimeEnvironmentVariables: {
              RUST_LOG: "info",
            },
          },
        },
        autoDeploymentsEnabled: false,
      },
      instanceConfiguration: {
        cpu: "256",
        memory: "512",
      },
      healthCheckConfiguration: {
        protocol: "HTTP",
        path: "/",
        interval: 10,
        timeout: 5,
        healthyThreshold: 1,
        unhealthyThreshold: 5,
      },
      autoScalingConfigurationArn: autoScaling.arn,
    });

    // DNS is managed via Cloudflare, just return App Runner URL
    return {
      url: appRunner.serviceUrl,
      ecrRepo: repo.repositoryUrl,
    };
  },
});
