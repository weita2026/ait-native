from __future__ import annotations

import importlib

import ait.cli as ait_cli
import ait_agent.cli as agent_cli
import ait_tk.launcher as ait_tk_launcher
import ait_native.cli as native_cli
import ait_native.common as native_common
import ait_chat.reply_config as chat_reply_config
import ait_chat.runtime_config as chat_runtime_config
import ait_chat.reply_attachments as chat_reply_attachments
import ait_chat.reply_context as chat_reply_context
import ait_chat.reply_http as chat_reply_http
import ait_native.local_content_bundle as native_local_content_bundle
import ait_native.local_content as native_local_content
import ait_native.local_content_workspace as native_local_content_workspace
import ait_native.local_control as native_local_control
import ait_native.packfiles as native_packfiles
import ait_native.read_models as native_read_models
import ait_native.remote_client as native_remote_client
import ait_native.repo_paths as native_repo_paths
import ait_native.server as native_server
import ait_native.server_auth as native_server_auth
import ait_native.server_content as native_server_content
import ait_native.server_control as native_server_control
import ait_native.server_db as native_server_db
import ait_native.server_paths as native_server_paths
import ait_native.server_queue as native_server_queue
import ait_native.server_store as native_server_store
import ait_native.store as native_store
import ait_native.store_local_workflow as native_store_local_workflow
import ait_native.store_worktrees as native_store_worktrees
import ait_native.treepacks as native_treepacks
import ait_native.web as native_web
import ait_native.worker as native_worker
import ait.local_content_bundle as ait_local_content_bundle
import ait.local_content as ait_local_content
import ait.local_content_workspace as ait_local_content_workspace
import ait.cli.bootstrap_views as ait_cli_bootstrap_views
import ait.cli.app_surfaces as ait_cli_app_surfaces
import ait.cli.task_dag_telegram_watch as ait_cli_task_dag_telegram_watch
import ait.cli.reply_runtime_seam as ait_cli_reply_runtime_seam
import ait.cli.server_runtime_helpers as ait_cli_server_runtime_helpers
import ait.cli.workflow_land_sync as ait_cli_workflow_land_sync
import ait.cli.workflow_land_batch as ait_cli_workflow_land_batch
import ait.cli.workflow_land_apply as ait_cli_workflow_land_apply
import ait.cli.workflow_land_command_hints as ait_cli_workflow_land_command_hints
import ait.cli.workflow_land_completed_local as ait_cli_workflow_land_completed_local
import ait.cli.workflow_land_publish as ait_cli_workflow_land_publish
import ait.cli.workflow_land_state as ait_cli_workflow_land_state
import ait.cli.workflow_land_text as ait_cli_workflow_land_text
import ait.cli.workflow_land_snapshot_replay as ait_cli_workflow_land_snapshot_replay
import ait.cli.workflow_land_selection as ait_cli_workflow_land_selection
import ait.cli.workflow_land_task_dag as ait_cli_workflow_land_task_dag
import ait.cli.workflow_land_views as ait_cli_workflow_land_views
import ait.cli.plan_publish_helpers as ait_cli_plan_publish_helpers
import ait.cli.plan_sync_matching as ait_cli_plan_sync_matching
import ait.cli.plan_sync_adoption as ait_cli_plan_sync_adoption
import ait.cli.plan_sync_scope as ait_cli_plan_sync_scope
import ait.cli.plan_markdown_authoring as ait_cli_plan_markdown_authoring
import ait.cli.review_submission_helpers as ait_cli_review_submission_helpers
import ait.cli.runtime_inspection_views as ait_cli_runtime_inspection_views
import ait.cli.line_transport_helpers as ait_cli_line_transport_helpers
import ait.cli.init_command as ait_cli_init_command
import ait.cli.workflow_identity_helpers as ait_cli_workflow_identity_helpers
import ait.cli.queue_summary_helpers as ait_cli_queue_summary_helpers
import ait.cli.task_close_tracking as ait_cli_task_close_tracking
import ait.cli.task_dag_command_payloads as ait_cli_task_dag_command_payloads
import ait.cli.task_dag_compact_packet_authoring as ait_cli_task_dag_compact_packet_authoring
import ait.cli.task_dag_execute_contract as ait_cli_task_dag_execute_contract
import ait.cli.task_dag_graph_artifacts as ait_cli_task_dag_graph_artifacts
import ait.cli.task_dag_compact_worker_runtime as ait_cli_task_dag_compact_worker_runtime
import ait.cli.task_dag_node_bootstrap as ait_cli_task_dag_node_bootstrap
import ait.cli.task_dag_execute_run_controls as ait_cli_task_dag_execute_run_controls
import ait.cli.task_dag_execute_run_state as ait_cli_task_dag_execute_run_state
import ait.cli.task_dag_readiness_views as ait_cli_task_dag_readiness_views
import ait.cli.session_runtime_helpers as ait_cli_session_runtime_helpers
import ait.cli.task_dag_runtime_helpers as ait_cli_task_dag_runtime_helpers
import ait.cli.task_dag_topology_helpers as ait_cli_task_dag_topology_helpers
import ait.cli.task_worktree_runtime as ait_cli_task_worktree_runtime
import ait.cli.workspace_command_locking as ait_cli_workspace_command_locking
import ait.cli.workflow_boundary_sessions as ait_cli_workflow_boundary_sessions
import ait.cli.workflow_authoring as ait_cli_workflow_authoring
import ait.local_control as ait_local_control
import ait.local_workflow_identity as ait_local_workflow_identity
import ait.local_workflow_sessions as ait_local_workflow_sessions
import ait.remote_client as ait_remote_client
import ait.repo_paths as ait_repo_paths
import ait.server_runtime_preflight as local_server_runtime_preflight
import ait.store as ait_store
import ait.store_line_cleanup as ait_store_line_cleanup
import ait.store_local_views as ait_store_local_views
import ait.store_repo_config as ait_store_repo_config
import ait.store_stash as ait_store_stash
import ait.store_workspace_replay as ait_store_workspace_replay
import ait.store_workspace_restore as ait_store_workspace_restore
import ait.store_worktree_bindings as ait_store_worktree_bindings
import ait.store_worktree_cleanup as ait_store_worktree_cleanup
import ait.store_worktree_filesystem as ait_store_worktree_filesystem
import ait.store_worktree_layout as ait_store_worktree_layout
import ait.store_local_workflow as ait_store_local_workflow
import ait.store_worktree_metadata as ait_store_worktree_metadata
import ait.store_worktree_runtime as ait_store_worktree_runtime
import ait.store_worktree_restore as ait_store_worktree_restore
import ait.store_worktree_lifecycle as ait_store_worktree_lifecycle
import ait.store_worktrees as ait_store_worktrees
import ait_protocol.common as protocol_common
import ait_protocol.runtime_roots as protocol_runtime_roots
import ait_agent.runtime_backend as agent_runtime_backend
import ait_agent.runtime_bindings as agent_runtime_bindings
import ait_agent.telegram.background_sync as telegram_background_sync
import ait_agent.telegram.config as telegram_config
import ait_agent.telegram.transport_io as telegram_transport_io
import ait_agent.telegram.workflow_queries as telegram_workflow_queries
import ait.server_runtime_seam as local_server_runtime_seam
import ait_server.agent_transport_runtime as server_agent_transport_runtime
import ait_server.server_auth as server_auth
import ait_server.server_content as server_content
import ait_server.server_content_storage as server_content_storage
import ait_server.server_control as server_control
import ait_server.server_db as server_db
import ait_server.server_paths as server_paths
import ait_server.server_queue as server_queue
import ait_server.server_store as server_store
import ait_server.store.land_request_payloads as server_store_land_request_payloads
import ait_server.store.plans as server_store_plans
import ait_server.store.releases as server_store_releases
import ait_server.store.sessions as server_store_sessions
import ait_server.store.task_tracking as server_store_task_tracking
import ait_server.store.stacks as server_store_stacks
import ait_storage.packfiles as storage_packfiles
import ait_storage.treepacks as storage_treepacks
import ait_server.app as server_app
import ait_server.local_repo_seams as server_local_repo_seams
import ait_server.server_process_runtime as server_process_runtime
import ait_server.read_models as server_read_models
import ait_server.repository_routes as server_repository_routes
import ait_server.route_request_models as server_route_request_models
import ait_server.stack_routes as server_stack_routes
import ait_server.task_routes as server_task_routes
import ait_server.task_dag_route_helpers as server_task_dag_route_helpers
import ait_server.session_route_helpers as server_session_route_helpers
import ait_server.session_transport_payloads as server_session_transport_payloads
import ait_server.task_dag_seams as server_task_dag_seams
import ait_server.worker as server_worker
import ait_web.app as web_app
import ait_web.agent_transport_runtime as web_agent_transport_runtime
import ait_web.chat_session_runtime as web_chat_session_runtime
import ait_web.server_auth_runtime as web_server_auth_runtime
import ait_web.server_context_runtime as web_server_context_runtime
import ait_web.server_read_runtime as web_server_read_runtime
import ait_web.server_write_runtime as web_server_write_runtime
import ait_web.telegram_transport_runtime as web_telegram_transport_runtime
import ait_web.telegram_workflow_runtime as web_telegram_workflow_runtime
import ait_web.transport_binding_runtime as web_transport_binding_runtime
import ait_web.clients as web_clients
import ait_web.clients.config as web_client_config
import ait_web.clients.read_api as web_client_read_api
import ait_web.clients.runtime as web_client_runtime
import ait_web.clients.sessions as web_client_sessions
import ait_web.clients.settings as web_client_settings
import ait_web.clients.write_api as web_client_write_api
import ait_web.local_repo_runtime as web_local_repo_runtime
import ait_web.pages as web_pages
import ait_web.pages.details as web_detail_pages
import ait_web.pages.queue as web_queue_pages
import ait_web.pages.repositories as web_repository_pages
import ait_web.pages.sessions as web_session_pages
import ait_web.pages.settings as web_settings_pages
import ait_web.rendering as web_rendering
import ait_web.rendering.html as web_rendering_html
import ait_web.rendering.query as web_rendering_query
import ait_web.rendering.theme_chat_shell as web_rendering_theme_chat_shell
import ait_web.rendering.theme_plan_board as web_rendering_theme_plan_board
import ait_web.rendering.theme as web_rendering_theme
import ait_web.routes as web_routes
import ait_web.routes.details as web_detail_routes
import ait_web.routes.queue as web_queue_routes
import ait_web.routes.repositories as web_repository_routes
import ait_web.routes.sessions as web_session_routes
import ait_web.routes.settings as web_settings_routes
import ait_web.routes.workflow_actions as web_workflow_action_routes


def test_directory_split_packages_expose_new_product_roots():
    agent_server_runtime_seam = importlib.reload(importlib.import_module("ait_agent.server_runtime_seam"))

    assert ait_cli.app is native_cli.app
    assert callable(agent_cli.main)
    assert callable(ait_tk_launcher.main)
    assert chat_reply_config.__file__
    assert callable(chat_reply_config.load_reply_generation_config)
    assert callable(chat_runtime_config.resolve_reply_runtime_env_path)
    assert callable(chat_runtime_config.load_runtime_env_file)
    assert chat_reply_context.__file__
    assert callable(chat_reply_context.prompt_messages_for_ai)
    assert chat_reply_attachments.__file__
    assert callable(chat_reply_attachments.extract_discord_reply_attachments)
    assert chat_reply_http.__file__
    assert callable(chat_reply_http.json_request)
    assert native_server is server_app
    assert native_read_models is server_read_models
    assert native_worker is server_worker
    assert server_route_request_models.__file__
    assert server_app.RepositoryCreate is server_route_request_models.RepositoryCreate
    assert server_app.PatchsetPublish is server_route_request_models.PatchsetPublish
    assert native_web.create_web_app is web_app.create_web_app
    assert native_web.main is web_app.main
    assert native_common is protocol_common
    assert native_local_content_bundle is ait_local_content_bundle
    assert native_local_content is ait_local_content
    assert native_local_content_workspace is ait_local_content_workspace
    assert native_local_control is ait_local_control
    assert ait_local_workflow_identity.__file__
    assert callable(ait_local_workflow_identity.workflow_sequence_from_id)
    assert callable(ait_local_workflow_identity.allocate_workflow_task_identity)
    assert callable(ait_local_workflow_identity.allocate_workflow_change_identity)
    assert ait_local_control.allocate_workflow_task_identity is ait_local_workflow_identity.allocate_workflow_task_identity
    assert ait_local_workflow_sessions.__file__
    assert callable(ait_local_workflow_sessions.create_workflow_session)
    assert callable(ait_local_workflow_sessions.create_workflow_checkpoint)
    assert callable(ait_local_workflow_sessions.record_workflow_snapshot_provenance)
    assert ait_local_control.create_workflow_session is ait_local_workflow_sessions.create_workflow_session
    assert native_remote_client is ait_remote_client
    assert native_repo_paths is ait_repo_paths
    assert native_store is ait_store
    assert local_server_runtime_preflight.__file__
    assert callable(local_server_runtime_preflight.postgres_preflight_report)
    assert native_store_local_workflow is ait_store_local_workflow
    assert native_store_worktrees is ait_store_worktrees
    assert ait_store.add_worktree is ait_store_worktrees.add_worktree
    assert ait_store.bind_worktree is ait_store_worktrees.bind_worktree
    assert ait_store.promote_worktree is ait_store_worktrees.promote_worktree
    assert ait_store.rebase_worktree is ait_store_worktrees.rebase_worktree
    assert ait_store.touch_worktree_usage is ait_store_worktrees.touch_worktree_usage
    assert ait_store.restore_owned_head is ait_store_worktrees.restore_owned_head
    assert ait_store_line_cleanup.__file__
    assert callable(ait_store_line_cleanup.list_line_cleanup_candidates)
    assert web_rendering_theme_chat_shell.__file__
    assert ".shared-chat-panel" in web_rendering_theme_chat_shell.CHAT_SHELL_CSS
    assert web_rendering_theme_chat_shell.CHAT_SHELL_CSS in web_rendering_theme.DEFAULT_CSS
    assert web_rendering_theme_plan_board.__file__
    assert ".sprint-board-grid" in web_rendering_theme_plan_board.PLAN_BOARD_CSS
    assert web_rendering_theme_plan_board.PLAN_BOARD_CSS in web_rendering_theme.DEFAULT_CSS
    assert native_packfiles is storage_packfiles
    assert native_treepacks is storage_treepacks
    assert native_server_auth is server_auth
    assert native_server_content is server_content
    assert server_content_storage.__file__
    assert callable(server_content_storage.snapshot_manifest_map)
    assert callable(server_content_storage.repository_storage_stats)
    assert callable(server_content_storage.repository_storage_signals)
    assert callable(server_content_storage.pack_repository)
    assert callable(server_content_storage.gc_repository_content)
    assert server_content.snapshot_manifest_map is server_content_storage.snapshot_manifest_map
    assert server_content.repository_storage_stats is server_content_storage.repository_storage_stats
    assert server_content.repository_storage_signals is server_content_storage.repository_storage_signals
    assert server_content.pack_repository is server_content_storage.pack_repository
    assert server_content.gc_repository_content is server_content_storage.gc_repository_content
    assert native_server_control is server_control
    assert native_server_db is server_db
    assert native_server_paths is server_paths
    assert native_server_queue is server_queue
    assert native_server_store is server_store
    assert server_store_land_request_payloads.__file__
    assert callable(server_store_land_request_payloads.land_request_payload)
    assert callable(server_store_land_request_payloads.land_freshness_result)
    assert server_store.create_planning_session is server_store_plans.create_planning_session
    assert server_store.list_planning_sessions is server_store_plans.list_planning_sessions
    assert server_store.get_planning_session is server_store_plans.get_planning_session
    assert server_store.append_planning_session_event is server_store_plans.append_planning_session_event
    assert server_store.list_planning_session_events is server_store_plans.list_planning_session_events
    assert server_store.join_planning_session is server_store_plans.join_planning_session
    assert server_store.promote_planning_session is server_store_plans.promote_planning_session
    assert server_store.close_planning_session is server_store_plans.close_planning_session
    assert server_store.publish_release is server_store_releases.publish_release
    assert server_store.get_release is server_store_releases.get_release
    assert server_store.get_release_for_repo is server_store_releases.get_release_for_repo
    assert server_store.read_release_artifact is server_store_releases.read_release_artifact
    assert server_store.create_session is server_store_sessions.create_session
    assert server_store.ensure_task_tracking_session is server_store_task_tracking.ensure_task_tracking_session
    assert server_store.backfill_task_tracking_sessions is server_store_task_tracking.backfill_task_tracking_sessions
    assert server_store.list_sessions is server_store_sessions.list_sessions
    assert server_store.create_stack is server_store_stacks.create_stack
    assert server_store.list_stacks is server_store_stacks.list_stacks

    assert callable(server_app.create_app)
    assert server_process_runtime.__file__
    assert callable(server_process_runtime._server_runtime_identity)
    assert callable(server_process_runtime._server_signal_stop_suffix)
    assert server_local_repo_seams.RepoContext is ait_repo_paths.RepoContext
    assert callable(server_local_repo_seams.load_config)
    assert callable(server_task_dag_seams.build_task_graph_progress)
    assert callable(server_task_dag_seams.render_task_dag_conversation_progress)
    assert server_task_dag_route_helpers.__file__
    assert callable(server_task_dag_route_helpers._task_dag_progress_summary_for_turn_impl)
    assert callable(server_task_dag_route_helpers._schedule_task_dag_notification)
    assert server_agent_transport_runtime.__file__
    assert callable(server_agent_transport_runtime.trigger_task_dag_telegram_notifications)
    assert server_session_transport_payloads.__file__
    assert callable(server_session_transport_payloads.build_session_assistant_reply_payload)
    assert server_session_route_helpers.__file__
    assert callable(server_session_route_helpers._reply_generation_config)
    assert callable(server_session_route_helpers._session_with_telegram_runtime_state)
    assert local_server_runtime_seam.ServerContext is server_paths.ServerContext
    assert local_server_runtime_seam.resolve_server_runtime_root is server_paths.resolve_server_runtime_root
    assert local_server_runtime_seam.postgres_preflight is server_db.postgres_preflight
    assert callable(agent_runtime_backend.resolve_agent_runtime_target)
    assert callable(agent_runtime_bindings.RuntimeSurfaceBindingStore)
    assert telegram_background_sync.__file__
    assert callable(telegram_background_sync.TelegramBackgroundSyncManager)
    assert telegram_transport_io.__file__
    assert callable(telegram_transport_io._json_request)
    assert callable(telegram_transport_io.parse_webhook_payload)
    assert telegram_workflow_queries.__file__
    assert callable(telegram_workflow_queries.parse_command)
    assert callable(telegram_workflow_queries.detect_workflow_query)
    assert callable(agent_server_runtime_seam.ServerContext.from_env)
    assert callable(agent_server_runtime_seam.get_worktree)
    assert callable(agent_server_runtime_seam.resolve_bound_repo_root)
    assert callable(server_worker.process_one)
    assert callable(web_app.create_web_app)
    assert callable(server_read_models.change_detail)
    assert callable(ait_local_content_bundle.export_snapshot_bundle)
    assert callable(ait_local_content_bundle.import_snapshot_bundle)
    assert callable(ait_local_content_workspace.workspace_ignore_policy)
    assert callable(ait_local_content_workspace.workspace_runtime_root_hygiene)
    assert callable(ait_store_local_workflow.create_local_plan)
    assert callable(ait_store_local_workflow.create_local_change)
    assert callable(ait_store_local_workflow.create_local_session)
    assert ait_store_local_views.__file__
    assert callable(ait_store_local_views._local_plan_view)
    assert callable(ait_store_local_views._local_release_view)
    assert ait_store_repo_config.__file__
    assert callable(ait_store_repo_config.load_config)
    assert callable(ait_store_repo_config.save_policy)
    assert ait_store_worktree_cleanup.__file__
    assert callable(ait_store_worktree_cleanup.list_worktree_cleanup_candidates)
    assert callable(ait_store_worktree_cleanup.remove_worktree)
    assert ait_store_worktree_runtime.__file__
    assert callable(ait_store_worktree_runtime.current_line)
    assert callable(ait_store_worktree_runtime.create_snapshot)
    assert ait_store_stash.__file__
    assert callable(ait_store_stash.create_stash)
    assert callable(ait_store_stash.list_stashes)
    assert callable(ait_store_stash.get_stash)
    assert callable(ait_store_stash.apply_stash)
    assert callable(ait_store_stash.drop_stash)
    assert ait_store_workspace_restore.__file__
    assert callable(ait_store_workspace_restore.restore_workspace)
    assert callable(ait_store_workspace_restore.restore_workspace_paths)
    assert ait_store_workspace_replay.__file__
    assert callable(ait_store_workspace_replay.revert_snapshot)
    assert callable(ait_store_workspace_replay.replay_snapshot)
    assert callable(ait_store_workspace_replay.revert_change)
    assert callable(ait_store_workspace_replay.replay_change)
    assert ait_store_worktree_restore.__file__
    assert callable(ait_store_worktree_restore.recreate_worktree)
    assert ait_store_worktree_lifecycle.__file__
    assert callable(ait_store_worktree_lifecycle.add_worktree)
    assert callable(ait_store_worktree_lifecycle.bind_worktree)
    assert callable(ait_store_worktree_lifecycle.promote_worktree)
    assert callable(ait_store_worktree_restore.restore_owned_head)
    assert ait_store_worktrees.restore_owned_head is ait_store_worktree_restore.restore_owned_head
    assert ait_store_worktrees.sync_all_worktrees is ait_store_worktree_restore.sync_all_worktrees
    assert ait_store_worktrees.add_worktree is ait_store_worktree_lifecycle.add_worktree
    assert ait_store_worktrees.bind_worktree is ait_store_worktree_lifecycle.bind_worktree
    assert ait_store_worktree_bindings.__file__
    assert callable(ait_store_worktree_bindings.canonical_bound_task_id)
    assert callable(ait_store_worktree_bindings.guard_worktree_binding_task_lineage)
    assert ait_store_worktree_layout.__file__
    assert callable(ait_store_worktree_layout.ensure_main_seed_mirror)
    assert ait_store_worktree_filesystem.__file__
    assert callable(ait_store_worktree_filesystem._create_directory_link)
    assert callable(ait_store_worktree_filesystem._copy_seed_tree)
    assert ait_store_worktree_metadata.__file__
    assert callable(ait_store_worktree_metadata._default_line_name)
    assert callable(ait_store_worktree_metadata._configured_task_worktree_policy)
    assert callable(ait_store_worktrees.add_worktree)
    assert callable(ait_store_worktrees.rebase_worktree)
    assert callable(ait_store_worktrees.sync_all_worktrees)
    assert ait_cli_bootstrap_views.__file__
    assert callable(ait_cli_bootstrap_views._render_init_summary)
    assert callable(ait_cli_bootstrap_views._emit_task_creation_payload)
    assert ait_cli_init_command.__file__
    assert callable(ait_cli_init_command.register_init_command)
    assert ait_cli_app_surfaces.__file__
    assert callable(ait_cli_app_surfaces.register_cli_subapps)
    assert callable(ait_cli_app_surfaces._ctx)
    assert ait_cli_task_dag_telegram_watch.__file__
    assert callable(ait_cli_task_dag_telegram_watch._maybe_auto_register_task_dag_telegram_watch)
    assert ait_cli_reply_runtime_seam.__file__
    assert callable(ait_cli_reply_runtime_seam.generate_session_reply)
    assert callable(ait_cli_reply_runtime_seam.load_reply_generation_config)
    assert ait_cli_server_runtime_helpers.__file__
    assert callable(ait_cli_server_runtime_helpers.postgres_preflight_report)
    assert ait_cli_workflow_land_sync.__file__
    assert callable(ait_cli_workflow_land_sync._attach_local_land_sync)
    assert callable(ait_cli_workflow_land_sync._restore_repo_root_after_land)
    assert ait_cli_plan_sync_matching.__file__
    assert callable(ait_cli_plan_sync_matching._plan_matches_sync_artifact)
    assert callable(ait_cli_plan_sync_matching._plan_artifact_identity_key)
    assert ait_cli_plan_sync_adoption.__file__
    assert callable(ait_cli_plan_sync_adoption._remote_plan_summary_to_plan)
    assert callable(ait_cli_plan_sync_adoption._resolve_local_sync_plan_candidate)
    assert ait_cli_plan_sync_scope.__file__
    assert callable(ait_cli_plan_sync_scope._resolve_plan_sync_target)
    assert callable(ait_cli_plan_sync_scope._preserve_workspace_paths_for_plan_sync)
    assert ait_cli_plan_markdown_authoring.__file__
    assert callable(ait_cli_plan_markdown_authoring._resolve_plan_artifact_input)
    assert callable(ait_cli_plan_markdown_authoring._guard_markdown_task_dispatch)
    assert ait_cli_review_submission_helpers.__file__
    assert callable(ait_cli_review_submission_helpers._latest_patchset_id)
    assert callable(ait_cli_review_submission_helpers._review_action_result)
    assert callable(ait_cli_review_submission_helpers._request_team_review_result)
    assert ait_cli_workflow_identity_helpers.__file__
    assert callable(ait_cli_workflow_identity_helpers._aligned_remote_publish_identity_request)
    assert callable(ait_cli_workflow_identity_helpers._require_remote_workflow_identity_family)
    assert callable(ait_cli_workflow_identity_helpers._require_remote_identity)
    assert ait_cli_queue_summary_helpers.__file__
    assert callable(ait_cli_queue_summary_helpers._queue_remote_section)
    assert callable(ait_cli_queue_summary_helpers._queue_summary_payload)
    assert ait_cli_task_close_tracking.__file__
    assert callable(ait_cli_task_close_tracking._maybe_attach_task_tracking)
    assert callable(ait_cli_task_close_tracking._finalize_task_close_tracking)
    assert ait_cli_runtime_inspection_views.__file__
    assert callable(ait_cli_runtime_inspection_views._local_auth_snapshot)
    assert callable(ait_cli_runtime_inspection_views._storage_validation_view)
    assert callable(ait_cli_runtime_inspection_views._history_rows)
    assert callable(ait_cli_runtime_inspection_views._config_summary)
    assert ait_cli_line_transport_helpers.__file__
    assert callable(ait_cli_line_transport_helpers._push_line)
    assert callable(ait_cli_line_transport_helpers._pull_line)
    assert ait_cli_session_runtime_helpers.__file__
    assert callable(ait_cli_session_runtime_helpers._resolve_session_turn_remote_target)
    assert callable(ait_cli_session_runtime_helpers._start_session_command_tracking)
    assert callable(ait_cli_session_runtime_helpers._compact_dag_worker_session_turn_guard)
    assert ait_cli_task_dag_runtime_helpers.__file__
    assert callable(ait_cli_task_dag_runtime_helpers._task_dag_readiness_payload)
    assert callable(ait_cli_task_dag_runtime_helpers._task_dag_readiness_from_remote_inventory)
    assert callable(ait_cli_task_dag_runtime_helpers._task_dag_graph_for_remote)
    assert ait_cli_task_dag_execute_run_state.__file__
    assert callable(ait_cli_task_dag_execute_run_state._task_dag_graph_run_id)
    assert callable(ait_cli_task_dag_execute_run_state._task_dag_state_snapshot_payload)
    assert callable(ait_cli_task_dag_execute_run_state._task_dag_execute_run_summary)
    assert ait_cli_task_dag_compact_packet_authoring.__file__
    assert callable(ait_cli_task_dag_compact_packet_authoring._task_dag_compact_packet_surface_payload)
    assert callable(ait_cli_task_dag_compact_packet_authoring._task_dag_generate_compact_packet_artifacts)
    assert callable(ait_cli_task_dag_compact_packet_authoring._task_dag_load_comparison_evidence)
    assert ait_cli_task_dag_execute_contract.__file__
    assert callable(ait_cli_task_dag_execute_contract._task_dag_auto_continue_profile)
    assert callable(ait_cli_task_dag_execute_contract._task_dag_execute_contract)
    assert callable(ait_cli_task_dag_execute_contract._task_dag_execute_payload)
    assert ait_cli_task_dag_graph_artifacts.__file__
    assert callable(ait_cli_task_dag_graph_artifacts._task_dag_graph_path_family)
    assert callable(ait_cli_task_dag_graph_artifacts._load_task_dag_graph_for_plan)
    assert ait_cli_task_dag_compact_worker_runtime.__file__
    assert callable(ait_cli_task_dag_compact_worker_runtime._task_dag_compact_worker_reply_poll_timeout_seconds)
    assert callable(ait_cli_task_dag_compact_worker_runtime._task_dag_run_local_compact_worker_turn)
    assert callable(ait_cli_task_dag_compact_worker_runtime._task_dag_start_auto_compact_worker)
    assert ait_cli_task_dag_node_bootstrap.__file__
    assert callable(ait_cli_task_dag_node_bootstrap._task_dag_create_task_for_node)
    assert callable(ait_cli_task_dag_node_bootstrap._task_dag_create_batch_session)
    assert callable(ait_cli_task_dag_node_bootstrap._task_dag_bootstrap_node)
    assert ait_cli_task_dag_execute_run_controls.__file__
    assert callable(ait_cli_task_dag_execute_run_controls._task_dag_open_execute_run)
    assert callable(ait_cli_task_dag_execute_run_controls._task_dag_refresh_execute_run)
    assert callable(ait_cli_task_dag_execute_run_controls._task_dag_control_execute_run)
    assert ait_cli_task_dag_topology_helpers.__file__
    assert callable(ait_cli_task_dag_topology_helpers._task_dag_node_workflow_boundary)
    assert callable(ait_cli_task_dag_topology_helpers._task_dag_converged_output_node_ids)
    assert callable(ait_cli_task_dag_topology_helpers._task_dag_execution_only_node_ids)
    assert ait_cli_task_dag_command_payloads.__file__
    assert callable(ait_cli_task_dag_command_payloads._task_dag_graph_payload)
    assert callable(ait_cli_task_dag_command_payloads._task_dag_progress_summary_payload)
    assert callable(ait_cli_task_dag_command_payloads._task_dag_dispatchable_rows)
    assert ait_cli_task_dag_readiness_views.__file__
    assert callable(ait_cli_task_dag_readiness_views._task_dag_view_rows)
    assert callable(ait_cli_task_dag_readiness_views._task_dag_schedule_payload)
    assert callable(ait_cli_task_dag_readiness_views._task_dag_change_focus_policy)
    assert ait_cli_task_worktree_runtime.__file__
    assert callable(ait_cli_task_worktree_runtime._guard_active_root_worktree)
    assert callable(ait_cli_task_worktree_runtime._maybe_auto_create_task_worktree)
    assert callable(ait_cli_task_worktree_runtime._maybe_auto_remove_bound_worktree_after_land)
    assert ait_cli_workspace_command_locking.__file__
    assert callable(ait_cli_workspace_command_locking._workspace_command_lock)
    assert callable(ait_cli_workspace_command_locking._run_locked_task_bound_authoring_command)
    assert issubclass(ait_cli_workspace_command_locking.WorkspaceCommandBusyError, ValueError)
    assert ait_cli_workflow_boundary_sessions.__file__
    assert callable(ait_cli_workflow_boundary_sessions._resolve_remote_workflow_boundary_session)
    assert callable(ait_cli_workflow_boundary_sessions._append_remote_workflow_boundary_event)
    assert callable(ait_cli_workflow_boundary_sessions._remote_task_tracking_session_seed)
    assert ait_cli_workflow_land_command_hints.__file__
    assert callable(ait_cli_workflow_land_command_hints._workflow_land_command_hints)
    assert callable(ait_cli_workflow_land_command_hints._workflow_land_next_action)
    assert callable(ait_cli_workflow_land_command_hints._workflow_land_suggested_commands)
    assert ait_cli_workflow_land_batch.__file__
    assert callable(ait_cli_workflow_land_batch._workflow_land_batch_payload)
    assert callable(ait_cli_workflow_land_batch._workflow_land_batch_run)
    assert callable(ait_cli_workflow_land_batch._workflow_land_completed_local_payload)
    assert callable(ait_cli_workflow_land_batch._workflow_land_completed_local_apply)
    assert ait_cli_workflow_land_apply.__file__
    assert callable(ait_cli_workflow_land_apply._workflow_land_apply)
    assert ait_cli_workflow_land_completed_local.__file__
    assert callable(ait_cli_workflow_land_completed_local._workflow_land_completed_local_preview_state)
    assert callable(ait_cli_workflow_land_completed_local._workflow_land_apply_completed_local_entry)
    assert callable(ait_cli_workflow_land_completed_local._workflow_land_batch_ensure_remote_task)
    assert callable(ait_cli_workflow_land_completed_local._workflow_land_batch_ensure_remote_change)
    assert callable(ait_cli_workflow_land_completed_local._published_local_task_plan_linkage_for_remote)
    assert ait_cli_workflow_land_publish.__file__
    assert callable(ait_cli_workflow_land_publish._publish_patchset_from_current_line)
    assert callable(ait_cli_workflow_land_publish._workflow_refresh_patchset_for_land)
    assert callable(ait_cli_workflow_land_publish._workflow_publish_payload)
    assert ait_cli_workflow_land_state.__file__
    assert callable(ait_cli_workflow_land_state._workflow_land_payload)
    assert callable(ait_cli_workflow_land_state._workflow_code_review_summary_count)
    assert callable(ait_cli_workflow_land_state._workflow_review_lane_counts)
    assert ait_cli_workflow_land_text.__file__
    assert callable(ait_cli_workflow_land_text._render_workflow_land_text)
    assert ait_cli_workflow_land_snapshot_replay.__file__
    assert callable(ait_cli_workflow_land_snapshot_replay._patchset_publish_context)
    assert callable(ait_cli_workflow_land_snapshot_replay._workflow_land_batch_ensure_remote_patchset_for_landed_change)
    assert callable(ait_cli_workflow_land_snapshot_replay._replay_snapshot_delta_onto_parent_bundle)
    assert callable(ait_cli_workflow_land_snapshot_replay._workflow_land_batch_ensure_remote_target_line_base)
    assert ait_cli_workflow_land_selection.__file__
    assert callable(ait_cli_workflow_land_selection._workflow_batch_local_change_entries)
    assert callable(ait_cli_workflow_land_selection._workflow_land_batch_graph_run_selector)
    assert ait_cli_workflow_land_task_dag.__file__
    assert callable(ait_cli_workflow_land_task_dag._workflow_batch_task_dag_entry_metadata)
    assert callable(ait_cli_workflow_land_task_dag._workflow_batch_local_task_dag_session_row)
    assert callable(ait_cli_workflow_land_task_dag._workflow_land_batch_ensure_remote_task_dag_session)
    assert ait_cli_workflow_land_views.__file__
    assert callable(ait_cli_workflow_land_views._workflow_land_batch_item_status)
    assert callable(ait_cli_workflow_land_views._workflow_land_applied_action_summary)
    assert ait_cli_workflow_authoring.__file__
    assert callable(ait_cli_workflow_authoring._validate_local_scope)
    assert callable(ait_cli_workflow_authoring._create_task_record)
    assert ait_cli_plan_publish_helpers.__file__
    assert callable(ait_cli_plan_publish_helpers._local_plan_publish)
    assert callable(ait_cli_plan_publish_helpers._map_equivalent_remote_plan_revision_suffix)
    assert callable(server_repository_routes.register_repository_routes)
    assert callable(server_stack_routes.register_stack_routes)
    assert callable(server_task_routes.register_task_routes)
    assert web_clients.__file__
    assert web_pages.__file__
    assert web_routes.__file__
    assert callable(web_rendering.escape_html)
    assert callable(web_rendering_html.render_page)
    assert callable(web_rendering_query.safe_return_path)
    assert callable(web_queue_routes.register_queue_routes)
    assert callable(web_repository_routes.register_repository_routes)
    assert callable(web_detail_routes.register_detail_routes)
    assert callable(web_session_routes.register_session_routes)
    assert callable(web_settings_routes.register_settings_routes)
    assert callable(web_workflow_action_routes.register_workflow_action_routes)
    assert callable(web_queue_pages.render_inbox)
    assert callable(web_queue_pages.render_task_queue)
    assert callable(web_repository_pages.render_repositories)
    assert callable(web_repository_pages.render_repository_detail)
    assert callable(web_detail_pages.render_change)
    assert callable(web_detail_pages.render_task_detail)
    assert callable(web_detail_pages.render_stack)
    assert callable(web_session_pages.render_session_queue)
    assert callable(web_session_pages.render_session_detail_page)
    assert callable(web_settings_pages.render_settings)
    assert callable(web_client_config.discover_web_repo_ctx)
    assert callable(web_client_read_api.change_detail)
    assert callable(web_client_write_api.record_change_review)
    assert callable(web_client_runtime.create_server_context)
    assert callable(web_client_sessions.append_web_note)
    assert callable(web_client_settings.save_id_namespace_prefix)
    assert web_local_repo_runtime.RepoContext is ait_repo_paths.RepoContext
    assert callable(web_local_repo_runtime.load_config)
    assert callable(web_local_repo_runtime.get_remote)
    assert callable(web_agent_transport_runtime.RuntimeSurfaceBindingStore)
    assert callable(web_agent_transport_runtime.load_config_for_telegram_worker)
    assert web_chat_session_runtime.__file__
    assert callable(web_chat_session_runtime._shared_binding_for_repo)
    assert web_server_auth_runtime.__file__
    assert web_server_auth_runtime.ActorContext.__name__ == "ActorContext"
    assert web_server_context_runtime.__file__
    assert callable(web_server_context_runtime.initialize)
    assert web_server_read_runtime.__file__
    assert callable(web_server_read_runtime._task_dag_progress)
    assert web_server_write_runtime.__file__
    assert callable(web_server_write_runtime._append_session_event)
    assert web_transport_binding_runtime.__file__
    assert callable(web_transport_binding_runtime.binding_transport)
    assert web_telegram_transport_runtime.__file__
    assert callable(web_telegram_transport_runtime.TelegramApiClient)
    assert web_telegram_workflow_runtime.__file__
    assert callable(web_telegram_workflow_runtime.format_queue_summary)
    assert web_rendering_theme.DEFAULT_CSS.startswith("\n:root")
    telegram_worker_config = importlib.import_module("ait_agent.telegram.worker_config")
    assert telegram_worker_config.__file__
    assert callable(telegram_worker_config.load_config_for_telegram_worker)
    assert telegram_config.__file__
    assert callable(telegram_config.load_config)
    assert telegram_config.BotConfig.__name__ == "BotConfig"

    web_server_data_runtime = importlib.import_module("ait_web.server_data_runtime")
    assert web_server_data_runtime.__file__
    assert web_server_data_runtime.ServerContext is server_paths.ServerContext
    assert callable(web_server_data_runtime.initialize)
    assert callable(web_server_data_runtime._connect)

    web_server_entry_runtime = importlib.import_module("ait_web.server_entry_runtime")
    assert web_server_entry_runtime.__file__
    assert web_server_entry_runtime.ServerContext is server_paths.ServerContext
