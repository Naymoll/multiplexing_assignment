# Задание
## Сервер
Серверное приложение принимает клиентское подключение, обрабатывает поступающие параллельно запросы выделяя на каждый по 100-500 мс (задержка выбирается рандомно), возвращает ответ клиенту. Задача сервера обработать все запросы одного клиента максимально быстро. Сервер должен принимать не более пяти одновременных клиентских подключений. Клиенты сверх установленного лимита ждут освобождения сервера. 
По завершении сеанса связи (отключение клиента) программа протоколирует статистику подключения состоящую из: 
- количества поступивших запросов; 
- максимального, минимального и среднего времени обработки запроса; 
- общее время обслуживания клиента (время сеанса). 
Сервер работает и обслуживает клиентов до получения сигнала Ctrl-C. 
При завершении работы сервер закрывает порт прослушивания протоколирует общую статистику по: 
- количеству обработанных клиентских подключений; 
- максимальному, минимальному и среднему времени сеанса с клиентом; 
- количество клиентов, не дождавшихся обслуживания. 
 
 ## Клиент
Клиентское приложение после запуска устанавливает подключение к серверу, отправляет серверу N одновременных сообщений в режиме мультиплексирования (HTTP/2 multiplexing). Получив ответы на все отправленные сообщения, потеряв связь или не дождавшись подключения к серверу клиент завершает работу. 
При завершении работы программа протоколирует статистику, состоящую из: 
- количества сообщений на которые получен ответ; 
- максимального, минимального и среднего времени ответа сервером на сообщение; 
- общего время обмена данными с сервером. 
Максимальное время ожидания подключения к серверу 2 секунды. 
Параметр N передаётся в приложение через аргумент командной строки и ограничен диапазоном от 1 до 100. При передаче параметра вне установленного диапазона протоколировать ошибку и завершать работу приложения. 
 
# MSR
Minimum Supported Rust Version - Писал на stable 1.63

# P.S.
Изначально использовал `actix-web`. Но в виду специфики задачи решил перейти на что-то более низкоуровневое. Смотрел в сторону `hyper`, но `h2` мне показался более подходящим под данную задачу.  
