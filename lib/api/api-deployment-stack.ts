import {
  AccessLogFormat,
  Deployment,
  LogGroupLogDestination,
  MethodLoggingLevel,
  Stage,
} from "aws-cdk-lib/aws-apigateway";
import { Stack, StackProps } from "aws-cdk-lib";

import { APIStack } from "./api-stack";
import { Construct } from "constructs";
import { LogGroup } from "aws-cdk-lib/aws-logs";
import config from "../../config";

export class APIDeploymentStack extends Stack {
  constructor(scope: Construct, id: string, props: StackProps) {
    super(scope, id, props);

    const api = APIStack.getInstance(this);

    const deployment = new Deployment(
      this,
      config.app.name(`ApiDeployment-${config.api.version}`),
      { api }
    );
    deployment.addToLogicalId(new Date().getTime()); // force deployment to update

    const logsGroup = new LogGroup(
      this,
      config.app.name(`ApiLogs-${config.api.version}`),
      {
        logGroupName: config.app
          .name(`-api-${config.api.version}`)
          .toLowerCase(),
      }
    );

    new Stage(this, config.app.name(`ApiStage-${config.api.version}`), {
      stageName: config.api.version,
      deployment,
      dataTraceEnabled: true,
      tracingEnabled: true,
      loggingLevel: MethodLoggingLevel.INFO,
      accessLogDestination: new LogGroupLogDestination(logsGroup),
      accessLogFormat: AccessLogFormat.jsonWithStandardFields({
        caller: true,
        httpMethod: true,
        ip: true,
        protocol: true,
        requestTime: true,
        resourcePath: true,
        responseLength: true,
        status: true,
        user: true,
      }),
    });
  }
}
