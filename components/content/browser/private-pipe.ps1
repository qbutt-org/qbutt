param([Parameter(Mandatory)][string]$PipeName)

$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

# Windows PowerShell's .NET Framework overload supplies an explicit DACL. A
# default libuv pipe DACL also grants Everyone read access, unsuitable for cookies.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.IO;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;

public static class QbuttBrowserPipe
{
    const int Limit = 768 * 1024;
    static readonly ConcurrentDictionary<int, NamedPipeServerStream> Clients = new ConcurrentDictionary<int, NamedPipeServerStream>();
    static readonly object OutputLock = new object();
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer { MaxJsonLength = Limit * 2 };
    static int nextId;
    static volatile bool closing;
    static readonly ManualResetEvent Ready = new ManualResetEvent(false);

    static void Emit(object value)
    {
        lock (OutputLock) { Console.Out.WriteLine(Json.Serialize(value)); Console.Out.Flush(); }
    }

    static bool ReadExactly(Stream stream, byte[] buffer)
    {
        int offset = 0;
        while (offset < buffer.Length)
        {
            int count = stream.Read(buffer, offset, buffer.Length - offset);
            if (count == 0) return false;
            offset += count;
        }
        return true;
    }

    static void ReadClient(int id, NamedPipeServerStream pipe)
    {
        try
        {
            byte[] header = new byte[4];
            while (ReadExactly(pipe, header))
            {
                int length = BitConverter.ToInt32(header, 0);
                if (length <= 0 || length > Limit) break;
                byte[] body = new byte[length];
                if (!ReadExactly(pipe, body)) break;
                Emit(new { connection = id, frame = Encoding.UTF8.GetString(body) });
            }
        }
        catch (IOException) { }
        catch (ObjectDisposedException) { }
        finally
        {
            NamedPipeServerStream removed;
            Clients.TryRemove(id, out removed);
            pipe.Dispose();
            Emit(new { connection = id, closed = true });
        }
    }

    static void Accept(string name, PipeSecurity security)
    {
        while (!closing)
        {
            NamedPipeServerStream pipe = null;
            try
            {
                pipe = new NamedPipeServerStream(name, PipeDirection.InOut, 4, PipeTransmissionMode.Byte,
                    PipeOptions.Asynchronous, 4096, 4096, security);
                Ready.Set();
                pipe.WaitForConnection();
                if (closing) { pipe.Dispose(); break; }
                int id = Interlocked.Increment(ref nextId);
                Clients[id] = pipe;
                NamedPipeServerStream connected = pipe;
                Task.Run(() => ReadClient(id, connected));
            }
            catch (IOException) { if (pipe != null) pipe.Dispose(); Thread.Sleep(50); }
        }
    }

    public static void Run(string name)
    {
        PipeSecurity security = new PipeSecurity();
        security.SetAccessRuleProtection(true, false);
        security.SetOwner(WindowsIdentity.GetCurrent().User);
        security.AddAccessRule(new PipeAccessRule(WindowsIdentity.GetCurrent().User,
            PipeAccessRights.FullControl, AccessControlType.Allow));
        Task.Run(() => Accept(name, security));
        if (!Ready.WaitOne(10000)) throw new IOException("Browser pipe startup timed out");
        Emit(new { ready = true });
        string line;
        while ((line = Console.In.ReadLine()) != null)
        {
            if (line.Length > Limit * 2) break;
            try
            {
                var value = Json.Deserialize<System.Collections.Generic.Dictionary<string, object>>(line);
                int id = Convert.ToInt32(value["connection"]);
                NamedPipeServerStream pipe;
                if (!Clients.TryGetValue(id, out pipe)) continue;
                if (value.ContainsKey("close")) { pipe.Dispose(); continue; }
                string frame = (string)value["frame"];
                byte[] body = Encoding.UTF8.GetBytes(frame);
                if (body.Length == 0 || body.Length > Limit) { pipe.Dispose(); continue; }
                lock (pipe)
                {
                    byte[] header = BitConverter.GetBytes(body.Length);
                    pipe.Write(header, 0, 4);
                    pipe.Write(body, 0, body.Length);
                    pipe.Flush();
                }
            }
            catch (IOException) { }
            catch (ObjectDisposedException) { }
        }
        closing = true;
        foreach (var pipe in Clients.Values) pipe.Dispose();
    }
}
'@ -ReferencedAssemblies System.Web.Extensions

[QbuttBrowserPipe]::Run($PipeName)
