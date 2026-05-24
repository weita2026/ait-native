from __future__ import annotations

from ait_agent.envelope import (
    build_transport_binding_metadata,
    build_transport_event_envelope,
    build_transport_reply_envelope,
    build_transport_session_metadata,
)


def test_build_transport_session_metadata_normalizes_generic_transport_fields():
    payload = build_transport_session_metadata(
        transport="slack",
        channel_id="C123",
        channel_title="release-room",
        channel_kind="channel",
        thread_id="T456",
        metadata_extra={"source": "slack", "linked_via": "ait-agent slack"},
    )

    assert payload == {
        "source": "slack",
        "transport": "slack",
        "transport_channel_id": "C123",
        "transport_channel_title": "release-room",
        "transport_channel_kind": "channel",
        "transport_thread_id": "T456",
        "linked_via": "ait-agent slack",
    }


def test_build_transport_binding_metadata_tracks_canonical_branch_and_reply_target():
    payload = build_transport_binding_metadata(
        transport="discord",
        surface_id="C123",
        surface_title="ops",
        surface_kind="thread",
        thread_id="TH999",
        binding_role="branch",
        canonical_session_id="AITS-0001",
        active_session_id="AITS-0002",
        branch_session_id="AITS-0002",
        branch_kind="planning",
        relink_reason="planning_mode_trigger",
        reply_target={"channel_id": "C123", "thread_id": "TH999", "message_id": 9},
    )

    assert payload == {
        "shared_session_transport": "discord",
        "shared_session_surface_id": "C123",
        "shared_session_surface_title": "ops",
        "shared_session_surface_kind": "thread",
        "shared_session_thread_id": "TH999",
        "shared_session_binding_role": "branch",
        "shared_session_canonical_session_id": "AITS-0001",
        "shared_session_active_session_id": "AITS-0002",
        "shared_session_branch_session_id": "AITS-0002",
        "shared_session_branch_kind": "planning",
        "shared_session_relink_reason": "planning_mode_trigger",
        "shared_session_transport_reply_target": {
            "channel_id": "C123",
            "thread_id": "TH999",
            "message_id": 9,
        },
    }


def test_build_transport_event_envelope_normalizes_actor_channel_and_logical_turn_ids():
    payload = build_transport_event_envelope(
        transport="telegram",
        actor_identity="telegram:456:@weita",
        actor_transport_id=456,
        actor_username="weita",
        actor_display_name="Wei Ta",
        channel_id=123,
        channel_title="Wei",
        channel_kind="private",
        text="hello",
        message_id=11,
        message_ids=[10, 11, 11, 0],
    )

    assert payload["schema_version"] == 1
    assert payload["transport"] == "telegram"
    assert payload["actor"] == {
        "actor_identity": "telegram:456:@weita",
        "transport_user_id": "456",
        "username": "weita",
        "display_name": "Wei Ta",
    }
    assert payload["channel"] == {
        "channel_id": "123",
        "channel_title": "Wei",
        "channel_kind": "private",
    }
    assert payload["message"] == {
        "text": "hello",
        "message_id": 11,
        "message_ids": [10, 11],
        "logical_turn_message_count": 2,
    }
    assert payload["event_id"] == "telegram:123:message:10-11"
    assert payload["dedupe_key"] == "telegram:123:message:10-11"


def test_build_transport_event_envelope_keeps_music_attachment_metadata():
    payload = build_transport_event_envelope(
        transport="telegram",
        actor_identity="telegram:456:@weita",
        channel_id=123,
        text="Uploaded song",
        attachments=[
            {
                "kind": "audio",
                "media_kind": "music",
                "file_id": "tg-audio-001",
                "file_unique_id": "unique-audio-001",
                "file_name": "song.mp3",
                "mime_type": "audio/mpeg",
                "duration": 42,
                "file_size": 2048,
                "caption": "Uploaded song",
            }
        ],
    )

    assert payload["message"]["attachments"] == [
        {
            "kind": "audio",
            "media_kind": "music",
            "telegram_file_id": "tg-audio-001",
            "telegram_file_unique_id": "unique-audio-001",
            "file_name": "song.mp3",
            "mime_type": "audio/mpeg",
            "caption": "Uploaded song",
            "duration_seconds": 42,
            "file_size_bytes": 2048,
        }
    ]


def test_build_transport_reply_envelope_tracks_target_and_reply_to_lineage():
    payload = build_transport_reply_envelope(
        transport="discord",
        channel_id="C123",
        channel_title="ops",
        channel_kind="thread",
        thread_id="TH999",
        text="status ok",
        reply_to_event_id="discord:C123:message:9",
        reply_to_message_id=9,
        reply_to_message_ids=[8, 9],
        metadata={"delivered_via": "discord_live"},
    )

    assert payload == {
        "schema_version": 1,
        "transport": "discord",
        "delivery_kind": "chat_reply",
        "target": {
            "channel_id": "C123",
            "channel_title": "ops",
            "channel_kind": "thread",
            "thread_id": "TH999",
        },
        "reply_to": {
            "event_id": "discord:C123:message:9",
            "message_id": 9,
            "message_ids": [8, 9],
            "logical_turn_message_count": 2,
        },
        "message": {"text": "status ok"},
        "metadata": {"delivered_via": "discord_live"},
    }


def test_build_transport_reply_envelope_keeps_attachment_delivery_hints():
    payload = build_transport_reply_envelope(
        transport="telegram",
        channel_id="123",
        text="Here is the track.",
        attachments=[
            {
                "kind": "audio",
                "file_name": "song.mp3",
                "mime_type": "audio/mpeg",
                "path": "/tmp/song.mp3",
                "title": "Song",
            }
        ],
    )

    assert payload["message"]["attachments"] == [
        {
            "kind": "audio",
            "file_name": "song.mp3",
            "mime_type": "audio/mpeg",
            "local_path": "/tmp/song.mp3",
            "title": "Song",
        }
    ]
