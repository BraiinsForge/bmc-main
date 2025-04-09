import type { Meta, StoryObj } from '@storybook/react';

import { Highlight as Component } from './Highlight';

const meta: Meta<typeof Component> = {
    title: 'components/Highlight',
    component: Component,
    args: {
        copy: true,
        diff: true,
    },
};

export default meta;
type Story = StoryObj<typeof Component>;

const tomlDiff =
    '  [telemetry]\n  enableFarmMetricsTelemetry = true\n  \n  [[server]]\n  name = "S1"\n  port = 3_336\n  \n- [[target]]\n- name = "SP"\n- url = "stratum+tcp://stratum.braiins.com:3333"\n- user_identity = "userName.workerName"\n- \n- [[routing]]\n- name = "RD"\n- from = [ "S1" ]\n+ [[server]]\n+ name = "S2"\n+ port = 3_333\n+ [[target]]\n+ name = "SP"\n+ url = "stratum+tcp://stratum.braiins.com:3333"\n+ user_identity = "userName.workerName"\n  \n- [[routing.goal]]\n- name = "Primary Goal"\n- \n- [[routing.goal.level]]\n- targets = [ "SP" ]\n- \n+ [[routing]]\n+ name = "RD"\n+ from = [ "S1" ]\n+ [[routing.goal]]\n+ name = "Primary Goal"\n  \n- \n- \n+ [[routing.goal.level]]\n+ targets = [ "SP" ]\n';

const toml =
    '  [telemetry]\n  enableFarmMetricsTelemetry = true\n  \n  [[server]]\n  name = "S1"\n  port = 3_336\n  \n  [[target]]\n  name = "SP"\n  url = "stratum+tcp://stratum.braiins.com:3333"\n  user_identity = "userName.workerName"\n  \n  [[routing]]\n  name = "RD"\n  from = [ "S1" ]\n  \n  [[routing.goal]]\n  name = "Primary Goal"\n  \n  [[routing.goal.level]]\n  targets = [ "SP" ]\n';

export const Highlight: Story = {
    render: ({ diff, copy }) => {
        const src = diff ? tomlDiff : toml;
        return <Component diff={diff} copy={copy} lang="toml" src={src} />;
    },
};
