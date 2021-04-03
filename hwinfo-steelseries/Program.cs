using System;
using System.Collections.Generic;
using SensorMonHTTP;
using System.Net.Http;
using System.IO;
using Newtonsoft.Json;
using System.Threading.Tasks;
using System.Text;
using System.Threading;
using System.Net.Http.Headers;
using System.Net;
using Newtonsoft.Json.Linq;

namespace hwinfo_steelseiresOLED
{
    public class SteelSeries
    {
        public string Address { get; set; }
        public string EncryptedAddress { get; set; }
    }

    public class Pages
    {
        public string metadata = "game_metadata";
        public string bind_event = "bind_game_event";
        public string register_event = "register_game_event";
        public string game_event = "game_event";
        public string game_heartbeat = "game_heartbeat";
        public string remove_game = "remove_game";
        public string remove_event = "remove_game_event";
    }

    public static class StringClean
    {
        public static string Clean(this string s)
        {
            // https://github.com/SteelSeries/gamesense-sdk/blob/master/doc/api/sending-game-events.md#game-events
            // https://stackoverflow.com/a/1321343
            // The values for game and event are limited to uppercase A-Z, 0-9, hyphen, and underscore characters.
            StringBuilder sb = new(s);
            sb.Replace(")", "-");
            sb.Replace("(", "-");
            sb.Replace(": ", "-");
            sb.Replace(" ", "_");
            // Removing bad characters
            sb.Replace("[", "");
            sb.Replace("]", "");
            sb.Replace(".", "");
            sb.Replace("#", "");

            return sb.ToString().ToUpper();
        }
    }

    class Program
    {
        public const string GAME = "HWINFO";
        static void Main(string[] args)
        {   
            Dictionary<string, Dictionary<string, List<string>>> Readings = GetHwinfoData();
            List<string> keys = new List<string>();
            foreach (string sensor in Readings.Keys)
            {
                keys.Add(sensor);
            }
            List<string> event_labels = new List<string>();
            
            foreach (string k in keys)
            {
                string cleaned = StringClean.Clean(k);
                event_labels.Add(cleaned);
            }
            #region HWiNFO Data
            //foreach (string sensor in Readings.Keys)
            //{
            //    Console.WriteLine(sensor);
            //    // Sensor == Event
            //    //  Create a new event for all sensors
            //    foreach (string key in Readings[sensor].Keys)
            //    {
            //        // Reading == Value
            //        //  For each of those sensors, create a new "value" for each row of readings
            //        Console.WriteLine("\t" + key);
            //        //Console.WriteLine("\t\tUnit\tCurrent\tMin\tMax\tAvg");
            //        //Console.Write("\t");
            //        // Reading[sensor][key][0] == Unit (MB/GB, W, RPM, MHz, V, °C, %, etc.)
            //        // Reading[sensor][key][1] == Current
            //        // Reading[sensor][key][2] == Minimum
            //        // Reading[sensor][key][3] == Maximum
            //        // Reading[sensor][key][4] == Average
            //        foreach (string item in Readings[sensor][key])
            //        {
            //            //Console.Write("\t" + item);
            //        }
            //        //Console.Write("\n");
            //    }
            //}

            #endregion

            string address = GetSSEAddress().Address;

            Pages pages = new Pages();

            //ImageBind(address, "IMAGE", pages);
            //SendEvent(address, "IMAGE", pages, "", new List<string> { "0","1","2","3","4" });

            //Console.WriteLine("Path:\n1 = Register, Bind, Send Event\n2 = Remove Event\n3 = Send only event data");
            //Console.Write("[ 1 / 2 / 3 ]: ");
            //string answer = Console.ReadLine();
            string answer = "1";
            // -------------------------
            if (answer == "1")
            {

                //int length = event_labels.Count;
                //for (int i = 0; i < length; i++)
                //{
                //    List<string> sensors = new List<string>();
                //    foreach (string sens in Readings[keys[i]].Keys)
                //    {
                //        sensors.Add(sens);
                //    }
                //    ThreeRowBind(address, event_labels[i], pages, sensors);
                //    break;
                //}
                //Thread.Sleep(2000);

                #region HandCrafted GPU info
                CraftedThreeRowBind(address, "GPU_TEMP", pages);
                CraftedThreeRowBind(address, "GPU_MHZ", pages);
                #endregion
                string mem_cur, mem_avg, hot_cur, hot_avg, clock_cur, clock_avg, memclock_cur, memclock_avg;
                bool speed = false;
                int counter = 0;
                #region DoWhile Repeating
                Console.WriteLine("Press ESC to stop");
                do
                {
                    while (!Console.KeyAvailable)
                    {
                        Readings = GetHwinfoData();
                        #region HandCrafted GPU Temp
                        Dictionary<string, List<string>> GPU = Readings[keys[13]];
                        mem_cur = GPU["GPU Memory Junction Temperature"][1];
                        mem_avg = GPU["GPU Memory Junction Temperature"][4];
                        hot_cur = GPU["GPU Hot Spot Temperature"][1];
                        hot_avg = GPU["GPU Hot Spot Temperature"][4];

                        clock_cur = GPU["GPU Clock"][1];
                        clock_avg = GPU["GPU Clock"][4];
                        memclock_cur = GPU["GPU Memory Clock"][1];
                        memclock_avg = GPU["GPU Memory Clock"][4];

                        if (speed)
                        {
                            SendGpuEvent(address, "GPU_TEMP", pages, mem_cur, mem_avg, hot_cur, hot_avg);
                            counter++;
                        }
                        else
                        {
                            SendGpuEvent(address, "GPU_MHZ", pages, clock_cur, clock_avg, memclock_cur, memclock_avg, true);
                            counter++;
                        }

                        if (counter > 5) // seconds to hold on one screen of data/
                        {
                            speed = !speed;
                            counter = 0;
                        }
                        #endregion
                        Thread.Sleep(1000);
                    }
                } while (Console.ReadKey(true).Key != ConsoleKey.Escape);
                #endregion
            }
            else if (answer == "2")
            {

                foreach (string k in event_labels)
                    RemoveEvent(address, pages, k);

                Thread.Sleep(1000);

                var removegame = new
                {
                    game = GAME,
                };
                var removegamestr = JsonConvert.SerializeObject(removegame);
                SendData(address, removegamestr, pages.remove_game);
            }
            else if (answer == "3")
            {
                //SendGpuTempEvent(address, "GPU_TEMP", pages);

                Thread.Sleep(1000);
            }


            //Console.Clear();
            //string[] arg = new string[0];
            //Main(arg);

            // -------------------------
            //Heartbeat(address, pages);

            // -------------------------
            
        }
        // -------------------------
        static SteelSeries GetSSEAddress()
        {
            string path = Environment.GetEnvironmentVariable("programdata") + "\\SteelSeries\\SteelSeries Engine 3\\coreProps.json";
            string file = File.ReadAllText(path);
            SteelSeries steelSeries = JsonConvert.DeserializeObject<SteelSeries>(file);
            return steelSeries;
        }
        // -------------------------
        static Dictionary<string, Dictionary<string, List<string>>> GetHwinfoData()
        {
            HWiNFOWrapper wrapper = new HWiNFOWrapper();
            Dictionary<string, Dictionary<string, List<string>>> Readings = wrapper.Open();
            //Dictionary<string, Dictionary<string, List<string>>> Readings = wrapper.Readings;
            wrapper.Close();
            return Readings;
        }
        // -------------------------
        static readonly HttpClientHandler httpClientHandler = new HttpClientHandler() { Proxy = new WebProxy("http://127.0.0.1:8080", false), UseProxy = false };
        static readonly HttpClient client = new HttpClient(httpClientHandler);
        // -------------------------
        static async void SendData(string address, string json, string page, bool waitForSuccess = true)
        {
            Uri uri = new Uri("http://" + address + "/" + page);
            HttpContent content = (HttpContent)new StringContent(json, Encoding.UTF8, "application/json");
            HttpResponseMessage response = await client.PostAsync(uri, content);
            if (waitForSuccess)
                response.EnsureSuccessStatusCode();
            //Console.WriteLine(response);
        }
        // -------------------------
        private static void RegisterEvent(string address, string @event, Pages pages)
        {
            var register_jsonData = new
            {
                game = GAME,
                @event = @event.ToUpper(),
                value_optional = true
            };
            string register_json = JsonConvert.SerializeObject(register_jsonData);
            SendData(address, register_json, pages.register_event);
        }
        // -------------------------

        public class Screen
        {
            public static string device_type {get { return "screened"; } }
            public string mode { get { return "screen"; } }
            public string zone { get { return "one"; } }
            private List<object> _datas = new List<object>();
            public List<object> datas
            {
                get { return _datas; }
                set { datas = _datas; }
            }
            public void Add(object a)
            {
                _datas.Add(a);
            }

        }
        private static void ImageBind(string address, string @event, Pages pages, int width = 128, int height = 48)
        {
            int size = width * height / 8;
            int[] array = new int[size];

            for (int i = 0; i < size; i++)
            {
                array[i] = 255;
            }


            var screen = new
            {
                device_type = "screened-128x48",
                datas = new List<object> {
                    new
                    {
                        has_text = false,
                        image_data = array
                    }
                }
            };

            var screen_obj = new
            {
                game = GAME,
                @event = @event,
                min_value = 0,
                max_value = 110,
                icon_id = 42,
                value_optional = true,
                handlers = new List<object> { screen }
            };

            string screen_json_obj = JsonConvert.SerializeObject(screen_obj)
                .Replace("device_type", "device-type") // Dealing with hyphen
                .Replace("has_text", "has-text")
                .Replace("image_data", "image-data");

            Console.WriteLine(screen_json_obj);

            SendData(address, screen_json_obj, pages.bind_event);
        }
        private static void ThreeRowBind(string address, string @event, Pages pages, List<string> sensors)
        {
            var screen_obj = new
            {
                game = GAME,
                @event = @event,
                min_value = 0,
                max_value = 110,
                icon_id = 42,
                value_optional = true,
                handlers = new List<object>()
                #region constructing list at initalize
                //handlers = new List<object>
                //{
                //    new
                //    {
                //        device_type = "screened",
                //        mode = "screen",
                //        zone = "one",
                //        datas = new List<object>
                //        {
                //            new
                //            {
                //                lines = new List<object>
                //                {
                //                    new
                //                    {
                //                        has_text = true,
                //                        context_frame_key = "Labels",
                //                        bold = true,
                //                        wrap = 0
                //                    },
                //                    new
                //                    {
                //                        has_text = true,
                //                        context_frame_key = "Row1",
                //                        wrap = 0
                //                    },
                //                    new
                //                    {
                //                        has_text = true,
                //                        context_frame_key = "Row2",
                //                        wrap = 0
                //                    }
                //                }
                //            }
                //        }
                //    }
                //}
                #endregion
            };

            //Screen scrn = new Screen();
            
            foreach (string sensor in sensors)
            {
                var new_obj = new
                {
                    device_type = "screened",
                    mode = "screen",
                    zone = "one",
                    datas = new List<object> {
                        new
                        {
                            lines = new List<object>
                            {
                                new
                                {
                                    has_text = true,
                                    context_frame_key = "Labels",
                                    bold = true,
                                    wrap = 0
                                },
                                new
                                {
                                    has_text = true,
                                    context_frame_key = "Row1",
                                    wrap = 0
                                },
                                new
                                {
                                    has_text = true,
                                    context_frame_key = "Row2",
                                    wrap = 0
                                }
                            }
                        }
                    }
                };

                screen_obj.handlers.Add(new_obj);
            }


            string screen_json_obj = JsonConvert.SerializeObject(screen_obj);
            screen_json_obj = screen_json_obj.Replace("device_type", "device-type") // Dealing with hyphen
                .Replace("context_frame_key", "context-frame-key")
                .Replace("has_text", "has-text");// Dealing with hyphen

            SendData(address, screen_json_obj, pages.bind_event);
        }
        private static void CraftedThreeRowBind(string address,string @event, Pages pages)
        {
            #region String format

            //string screen_json_str = @"
            //    {
            //        'game': 'HWINFO',
            //        'event': 'GPU_TEMP',
            //        'min_value':0,
            //        'max_value':110,
            //        'icon_id':42,
            //        'value_optional':true,
            //        'handlers': [{
            //            'device-type': 'screened',
            //            'mode': 'screen',
            //            'zone': 'one',
            //            'datas': [{
            //                'lines': [
            //                    {
            //                        'has-text':true,
            //                        'context-frame-key':'Labels',
            //                        'bold':true,
            //                        'wrap':0
            //                    },
            //                    {
            //                        'has-text':true,
            //                        'context-frame-key':'Row1',
            //                        'wrap':0
            //                    },
            //                    {
            //                        'has-text':true,
            //                        'context-frame-key':'Row2',
            //                        'wrap':0
            //                    }
            //                ]
            //            }]
            //        }]
            //    }";
            //var screen_json = JsonConvert.SerializeObject(JObject.Parse(screen_json_str));

            #endregion
            var screen_obj = new
            {
                game = GAME,
                @event = @event,
                min_value = 0,
                max_value = 110,
                icon_id = 42,
                value_optional = true,
                handlers = new List<object>
                {
                    new
                    {
                        device_type = "screened",
                        mode = "screen",
                        zone = "one",
                        datas = new List<object>
                        {
                            new
                            {

                                length_millis = 5000,
                                lines = new List<object>
                                {
                                    new
                                    {
                                        has_text = true,
                                        context_frame_key = "Labels",
                                        bold = true,
                                        wrap = 0
                                    },
                                    new
                                    {
                                        has_text = true,
                                        context_frame_key = "Row1",
                                        wrap = 0
                                    },
                                    new
                                    {
                                        has_text = true,
                                        context_frame_key = "Row2",
                                        wrap = 0
                                    }
                                }
                            }
                        }
                    }
                }
            };
            string screen_json_obj = JsonConvert.SerializeObject(screen_obj);
            screen_json_obj = screen_json_obj.Replace("device_type", "device-type") // Dealing with hyphen
                .Replace("context_frame_key", "context-frame-key")
                .Replace("has_text", "has-text")
                .Replace("length_millis","length-millis");// Dealing with hyphen

            SendData(address, screen_json_obj, pages.bind_event);
        }
        // ------------------------- °C
        private static void SendEvent(string address, string @event, Pages pages, string sensor, List<string> data)
        {
            var event_obj = new
            {
                game = GAME,
                @event = @event.ToUpper(),
                value_optional = true,
                data = new
                {
                    //value = 24,
                    frame = new
                    {
                        sensor = sensor,
                        Row1 = "Unit\tCurrent\tMin\tMax\tAvg",
                        Row2 = String.Format("{0}\t{1:F1}\t{2:F1}\t{3:F1}\t{4:F1}", data[0], data[1], data[2], data[3], data[4])
                    }
                }
            };
            string event_json_obj = JsonConvert.SerializeObject(event_obj);
            event_json_obj.Replace("image_data_128x48", "image-data-128x48");
            SendData(address, event_json_obj, pages.game_event);
        }
        private static void SendGpuEvent(string address, string @event, Pages pages, string row1_cur, string row1_avg, string row2_cur, string row2_avg, bool hurtz = false)
        {
            #region String format

            //string event_jsonData_str = @"
            //{
            //    'game': 'HWINFO',
            //    'event': 'GPU_TEMP',
            //    'data': {
            //        'value':24,
            //        'frame':{
            //            'Labels':'GPU\tCurr.\tAvg.',
            //            'Row1' :   'Memory 98°\t97.5°',
            //            'Row2' :  'Hotspot 68°\t67.5°'
            //        }
            //    }
            //}";
            //var event_json = JsonConvert.SerializeObject(JObject.Parse(event_jsonData_str));

            #endregion

            double drow1_cur = Convert.ToDouble(row1_cur);
            double drow1_avg = Convert.ToDouble(row1_avg);
            double drow2_cur = Convert.ToDouble(row2_cur);
            double drow2_avg = Convert.ToDouble(row2_avg);

            string row1_label, row2_label, unit, label;
            

            if (hurtz)
            {
                row1_label = "Clock";
                row2_label = "Mem  ";
                unit = "";
                label = "MHz";
            }
            else
            {
                row1_label = "Mem ";
                row2_label = "Hot   ";
                unit = "°C";
                label = "Temp";
            }
            int length = label.Length;

            var event_obj = new
            {
                game = GAME,
                @event = @event.ToUpper(),
                value_optional = true,
                data = new
                {
                    value = 24,
                    frame = new
                    {
                        Labels = String.Format("GPU  {0}  Avg.", label),
                        Row1 = String.Format("{2}{3} {0:F0}\t{1:F0}", drow1_cur, drow1_avg, row1_label, unit),
                        Row2 = String.Format("{2}{3} {0:F0}\t{1:F0}", drow2_cur, drow2_avg, row2_label, unit)
                    }
                }
            };
            string event_json_obj = JsonConvert.SerializeObject(event_obj);

            SendData(address, event_json_obj, pages.game_event);
        }
        private static void SendGpuMHzEvent(string address, string @event, Pages pages, string mem_cur, string mem_avg, string hot_cur, string hot_avg)
        {
            double dmem_cur = Convert.ToDouble(mem_cur);
            double dmem_avg = Convert.ToDouble(mem_avg);
            double dhot_cur = Convert.ToDouble(hot_cur);
            double dhot_avg = Convert.ToDouble(hot_avg);

            var event_obj = new
            {
                game = GAME,
                @event = @event.ToUpper(),
                value_optional = true,
                data = new
                {
                    value = 24,
                    frame = new
                    {
                        Labels = "GPU\tCurr.\tAvg.",
                        Row1 = String.Format("Core {0:0}° {1:0}°", dmem_cur, dmem_avg),
                        Row2 = String.Format("Memory {0:0}° {1:0}°", dhot_cur, dhot_avg)
                    }
                }
            };
            string event_json_obj = JsonConvert.SerializeObject(event_obj);

            SendData(address, event_json_obj, pages.game_event);
        }
        // -------------------------
        private static void RemoveEvent(string address, Pages pages, string @event)
        {
            var remove_event_jsonData = new
            {
                game = GAME,
                @event = @event
            };
            var remove_event_json = JsonConvert.SerializeObject(remove_event_jsonData);
            SendData(address, remove_event_json, pages.remove_event, false);
        }
        // -------------------------
        private static void Heartbeat(string address, Pages pages)
        {
            var heatbreak_jsonData = new
            {
                game = "HWINFO"
            };
            var heartbeat_json = JsonConvert.SerializeObject(heatbreak_jsonData);
            SendData(address, heartbeat_json, pages.game_heartbeat);
        }
    }
}
