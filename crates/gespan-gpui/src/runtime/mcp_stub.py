#!/usr/bin/env python3
# A stub provider's MCP client: real streamable HTTP against the tool endpoint,
# with the credential from thread/start's config, as Codex receives it.
import json, os, socket, sys, time, urllib.error, urllib.request
class Hub:
    def __init__(self, server):
        self.url = server['url']
        self.authorization = server['http_headers']['Authorization']
        self.session = None
        self.next_id = 10
        self.request('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'stub', 'version': '1'}})
        self.post({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
    def post(self, message):
        headers = {'Authorization': self.authorization, 'Accept': 'application/json, text/event-stream', 'Content-Type': 'application/json'}
        if self.session:
            headers['Mcp-Session-Id'] = self.session
            headers['MCP-Protocol-Version'] = '2025-11-25'
        request = urllib.request.Request(self.url, json.dumps(message).encode(), headers)
        return urllib.request.urlopen(request, timeout=30)
    def request(self, method, params=None):
        self.next_id += 1
        message = {'jsonrpc': '2.0', 'id': self.next_id, 'method': method}
        if params is not None:
            message['params'] = params
        with self.post(message) as response:
            self.session = response.headers.get('Mcp-Session-Id') or self.session
            for line in response:
                line = line.decode().strip()
                if line.startswith('data:') and line[5:].strip().startswith('{'):
                    reply = json.loads(line[5:])
                    if reply.get('id') == self.next_id:
                        return self.unwrap(reply)
                elif line.startswith('{'):
                    return self.unwrap(json.loads(line))
        raise RuntimeError(f'no reply to {method}')
    def unwrap(self, reply):
        if 'error' in reply:
            raise RuntimeError(reply['error']['message'])
        return reply['result']
    def tools(self):
        return self.request('tools/list')['tools']
    def call(self, name, arguments):
        result = self.request('tools/call', {'name': name, 'arguments': arguments})
        text = result['content'][0]['text']
        if result.get('isError'):
            return {'ok': False, 'error': text}
        return {'ok': True, 'result': json.loads(text)}
