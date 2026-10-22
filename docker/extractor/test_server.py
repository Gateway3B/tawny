from contextlib import contextmanager, redirect_stderr, redirect_stdout
from http.client import HTTPConnection, RemoteDisconnected
from io import StringIO
import json
import subprocess
import threading
import unittest
from unittest.mock import patch

import server


class CommandTests(unittest.TestCase):
    def test_command_has_token_provider_and_supported_js_runtime(self) -> None:
        command = server.build_video_command("aNXB-8Aqt88")
        self.assertIn("node", command)
        # Pinning a player client is what broke extraction once already: `mweb`
        # started returning "Video unavailable" and playback fell back to gated
        # URLs that die after about a minute. Let yt-dlp choose.
        self.assertFalse(
            any(value.startswith("youtube:player_client=") for value in command)
        )
        self.assertTrue(
            any(value.startswith("youtubepot-bgutilhttp:base_url=") for value in command)
        )

    def test_command_rejects_non_video_ids(self) -> None:
        with self.assertRaises(ValueError):
            server.build_video_command("../../secrets")

    def test_shorts_command_is_flat_and_bounded(self) -> None:
        command = server.build_channel_shorts_command("UCsXVk37bltHxD1rDPwtNM8Q")
        self.assertIn("--flat-playlist", command)
        self.assertIn("30", command)
        self.assertTrue(command[-1].endswith("/shorts"))

    def test_shorts_command_rejects_non_channel_ids(self) -> None:
        with self.assertRaises(ValueError):
            server.build_channel_shorts_command("veritasium")

    def test_upload_listing_keeps_dates_and_covers_the_complete_tab(self) -> None:
        command = server.build_channel_uploads_command("UCsXVk37bltHxD1rDPwtNM8Q", "videos")
        self.assertIn("--flat-playlist", command)
        self.assertIn("youtubetab:approximate_date", command)
        self.assertNotIn("--playlist-end", command)
        self.assertTrue(command[-1].endswith("/videos"))

    def test_upload_listing_rejects_unknown_tabs_and_channel_paths(self) -> None:
        for channel, tab in [("../../secret", "videos"), ("UCsXVk37bltHxD1rDPwtNM8Q", "../videos")]:
            with self.assertRaises(ValueError):
                server.build_channel_uploads_command(channel, tab)


class PrivacyLogTests(unittest.TestCase):
    @contextmanager
    def service(self):
        output, errors = StringIO(), StringIO()
        service = server.PrivateHTTPServer(("127.0.0.1", 0), server.Handler)
        thread = threading.Thread(target=service.serve_forever, daemon=True)
        with redirect_stdout(output), redirect_stderr(errors):
            thread.start()
            try:
                yield service.server_address[1], output, errors
            finally:
                service.shutdown()
                service.server_close()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())

    def test_failed_extraction_keeps_private_diagnostics_out_of_logs(self):
        result = subprocess.CompletedProcess([], 1, b"", b"provider-error-canary token-canary")
        with self.service() as (port, output, errors), patch.object(
            server, "provider_is_ready", return_value=True
        ), patch.object(server.subprocess, "run", return_value=result):
            connection = HTTPConnection("127.0.0.1", port, timeout=5)
            connection.request("GET", "/v1/videos/aNXB-8Aqt88?query-canary", headers={"Cookie": "cookie-canary"})
            response = connection.getresponse()
            self.assertEqual(response.status, 502)
            # The private internal response still lets Tawny classify failure.
            self.assertIn("provider-error-canary", json.loads(response.read())["ytdlp_error"])
            connection.close()
        self.assertEqual(errors.getvalue(), "")
        self.assertEqual([json.loads(line) for line in output.getvalue().splitlines()], [
            {"message": "service_event", "code": "extraction_failed"}
        ])

    def test_access_logs_do_not_retain_unknown_paths_or_request_details(self):
        with self.service() as (port, output, errors):
            connection = HTTPConnection("127.0.0.1", port, timeout=5)
            connection.request("GET", "/unknown-person-canary?token-canary", headers={"Authorization": "Bearer secret-canary"})
            response = connection.getresponse()
            self.assertEqual(response.status, 404)
            response.read()
            connection.close()
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(errors.getvalue(), "")

    def test_unexpected_handler_errors_never_print_traceback_or_address(self):
        with self.service() as (port, output, errors), patch.object(
            server, "provider_is_ready", return_value=True
        ), patch.object(server.subprocess, "run", side_effect=RuntimeError("exception-secret-canary")):
            connection = HTTPConnection("127.0.0.1", port, timeout=5)
            connection.request("GET", "/v1/videos/aNXB-8Aqt88")
            with self.assertRaises(RemoteDisconnected):
                connection.getresponse()
            connection.close()
        self.assertEqual(errors.getvalue(), "")
        self.assertEqual([json.loads(line) for line in output.getvalue().splitlines()], [
            {"message": "service_event", "code": "request_failed"}
        ])

    def test_bad_startup_configuration_is_reported_without_its_value(self):
        output, errors = StringIO(), StringIO()
        with patch.dict(server.os.environ, {"PORT": "private-config-canary"}), redirect_stdout(output), redirect_stderr(errors):
            with self.assertRaises(SystemExit) as stopped:
                server.main()
        self.assertEqual(stopped.exception.code, 1)
        self.assertEqual(errors.getvalue(), "")
        self.assertEqual(json.loads(output.getvalue()), {"message":"service_event", "code":"service_failed"})


if __name__ == "__main__":
    unittest.main()
