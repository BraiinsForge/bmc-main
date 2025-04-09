import type { Message } from '@bufbuild/protobuf';

export type PlainProtoMessage<Msg extends Message> = {
    [Key in keyof Msg as Exclude<Key, '$typeName' | '$unknown'>]: Msg[Key] extends Message
        ? PlainProtoMessage<Msg[Key]>
        : Msg[Key] extends Array<Message>
          ? Array<PlainProtoMessage<Msg[Key][number]>>
          : Msg[Key];
};

export type PartialMessage<T extends Message> = Partial<PlainProtoMessage<T>>;
