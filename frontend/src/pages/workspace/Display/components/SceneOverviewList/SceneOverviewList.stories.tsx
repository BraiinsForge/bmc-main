import { useState } from 'react';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import * as mock from '@/mocks';
import { SceneOverviewList as Component, type SceneOverviewListProps } from './SceneOverviewList';

export default {
    title: 'display/components/SceneOverviewList',
    component: Component,
} satisfies Meta<SceneOverviewListProps>;

const initialState: pb.Scene[] = [
    {
        $typeName: 'braiins.bmc.web.Scene',
        id: '0',
        enabled: true,
        cycleDurationSec: 10,
        kind: {
            case: 'fullscreen',
            value: {
                $typeName: 'braiins.bmc.web.Scene.Fullscreen',
                widget: {
                    $typeName: 'braiins.bmc.web.Widget',
                    id: mock.uuid(),
                    size: pb.WidgetSize.FULL,
                    position: {
                        $typeName: 'braiins.bmc.web.WidgetPosition',
                        row: 0,
                        col: 0,
                    },
                    kind: {
                        $typeName: 'braiins.bmc.web.WidgetKind',
                        value: {
                            case: 'clock',
                            value: {
                                $typeName: 'braiins.bmc.web.ClockWidget',
                                clockStyle: mock.proto.randomEnumItem(pb.ClockWidget_ClockStyle),
                                showDate: true,
                                showSeconds: true,
                                showTimezone: true,
                                timezone: 'UTC',
                                numbersFontStyle: mock.proto.randomEnumItem(pb.FontStyle),
                            },
                        },
                    },
                },
            },
        },
    } satisfies pb.Scene,
    {
        $typeName: 'braiins.bmc.web.Scene',
        id: '1',
        cycleDurationSec: 11,
        enabled: true,
        kind: {
            case: 'fullscreen',
            value: {
                $typeName: 'braiins.bmc.web.Scene.Fullscreen',
                widget: {
                    $typeName: 'braiins.bmc.web.Widget',
                    id: mock.uuid(),
                    size: pb.WidgetSize.FULL,
                    position: {
                        $typeName: 'braiins.bmc.web.WidgetPosition',
                        row: 0,
                        col: 0,
                    },
                    kind: {
                        $typeName: 'braiins.bmc.web.WidgetKind',
                        value: {
                            case: 'clock',
                            value: {
                                $typeName: 'braiins.bmc.web.ClockWidget',
                                clockStyle: mock.proto.randomEnumItem(pb.ClockWidget_ClockStyle),
                                showDate: true,
                                showSeconds: true,
                                showTimezone: true,
                                timezone: 'UTC',
                                numbersFontStyle: mock.proto.randomEnumItem(pb.FontStyle),
                            },
                        },
                    },
                },
            },
        },
    } satisfies pb.Scene,
    {
        $typeName: 'braiins.bmc.web.Scene',
        id: '2',
        enabled: true,
        cycleDurationSec: 11,
        kind: {
            case: 'fullscreen',
            value: {
                $typeName: 'braiins.bmc.web.Scene.Fullscreen',
                widget: {
                    $typeName: 'braiins.bmc.web.Widget',
                    id: mock.uuid(),
                    size: pb.WidgetSize.FULL,
                    position: {
                        $typeName: 'braiins.bmc.web.WidgetPosition',
                        row: 0,
                        col: 0,
                    },
                    kind: {
                        $typeName: 'braiins.bmc.web.WidgetKind',
                        value: {
                            case: 'clock',
                            value: {
                                $typeName: 'braiins.bmc.web.ClockWidget',
                                clockStyle: mock.proto.randomEnumItem(pb.ClockWidget_ClockStyle),
                                showDate: true,
                                showSeconds: true,
                                showTimezone: true,
                                timezone: 'UTC',
                                numbersFontStyle: mock.proto.randomEnumItem(pb.FontStyle),
                            },
                        },
                    },
                },
            },
        },
    } satisfies pb.Scene,
    {
        $typeName: 'braiins.bmc.web.Scene',
        id: '3',
        enabled: true,
        cycleDurationSec: 13,
        kind: {
            case: 'fullscreen',
            value: {
                $typeName: 'braiins.bmc.web.Scene.Fullscreen',
                widget: {
                    $typeName: 'braiins.bmc.web.Widget',
                    id: mock.uuid(),
                    size: pb.WidgetSize.FULL,
                    position: {
                        $typeName: 'braiins.bmc.web.WidgetPosition',
                        row: 0,
                        col: 0,
                    },
                    kind: {
                        $typeName: 'braiins.bmc.web.WidgetKind',
                        value: {
                            case: 'clock',
                            value: {
                                $typeName: 'braiins.bmc.web.ClockWidget',
                                clockStyle: mock.proto.randomEnumItem(pb.ClockWidget_ClockStyle),
                                showDate: true,
                                showSeconds: true,
                                showTimezone: true,
                                timezone: 'UTC',
                                numbersFontStyle: mock.proto.randomEnumItem(pb.FontStyle),
                            },
                        },
                    },
                },
            },
        },
    } satisfies pb.Scene,
    {
        $typeName: 'braiins.bmc.web.Scene',
        id: '4',
        enabled: true,
        cycleDurationSec: 14,
        kind: {
            case: 'fullscreen',
            value: {
                $typeName: 'braiins.bmc.web.Scene.Fullscreen',
                widget: {
                    $typeName: 'braiins.bmc.web.Widget',
                    id: mock.uuid(),
                    size: pb.WidgetSize.FULL,
                    position: {
                        $typeName: 'braiins.bmc.web.WidgetPosition',
                        row: 0,
                        col: 0,
                    },
                    kind: {
                        $typeName: 'braiins.bmc.web.WidgetKind',
                        value: {
                            case: 'clock',
                            value: {
                                $typeName: 'braiins.bmc.web.ClockWidget',
                                clockStyle: mock.proto.randomEnumItem(pb.ClockWidget_ClockStyle),
                                showDate: true,
                                showSeconds: true,
                                showTimezone: true,
                                timezone: 'UTC',
                                numbersFontStyle: mock.proto.randomEnumItem(pb.FontStyle),
                            },
                        },
                    },
                },
            },
        },
    } satisfies pb.Scene,
    {
        $typeName: 'braiins.bmc.web.Scene',
        id: '5',
        cycleDurationSec: 15,
        enabled: false,
        kind: {
            case: 'fullscreen',
            value: {
                $typeName: 'braiins.bmc.web.Scene.Fullscreen',
                widget: {
                    $typeName: 'braiins.bmc.web.Widget',
                    id: mock.uuid(),
                    size: pb.WidgetSize.FULL,
                    position: {
                        $typeName: 'braiins.bmc.web.WidgetPosition',
                        row: 0,
                        col: 0,
                    },
                    kind: {
                        $typeName: 'braiins.bmc.web.WidgetKind',
                        value: {
                            case: 'clock',
                            value: {
                                $typeName: 'braiins.bmc.web.ClockWidget',
                                clockStyle: mock.proto.randomEnumItem(pb.ClockWidget_ClockStyle),
                                showDate: true,
                                showSeconds: true,
                                showTimezone: true,
                                timezone: 'UTC',
                                numbersFontStyle: mock.proto.randomEnumItem(pb.FontStyle),
                            },
                        },
                    },
                },
            },
        },
    } satisfies pb.Scene,
];

function Demo() {
    const [scenes, setScenes] = useState<pb.Scene[]>(initialState);
    return (
        <Component
            scenes={scenes}
            onMove={setScenes}
            onEdit={action('onEdit')}
            onClone={action('onClone')}
            onDelete={action('onDelete')}
            onToggle={action('onToggle')}
            onDurationChange={action('onDurationChange')}
            cycleEnabled
            cycleDefaultDuration={30}
        />
    );
}

export function SceneOverviewList() {
    return <Demo />;
}
